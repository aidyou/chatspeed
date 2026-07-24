//! Workflow database operations
//!
//! This module provides database operations for managing workflows and their messages.

use crate::db::{MainStore, StoreError};
use crate::workflow::react::events::{WorkflowEvent, WorkflowEventRecord};
use crate::workflow::react::replay::replay_events_to_execution_context;
use crate::workflow::react::types::{ExecutionContext, RuntimeState, WaitReason};
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

fn estimate_ai_context_tokens(messages: &[WorkflowAiContextMessage]) -> usize {
    messages
        .iter()
        .map(|message| {
            let mut total =
                crate::ccproxy::utils::token_estimator::estimate_tokens(&message.message);
            if let Some(reasoning) = message.reasoning.as_deref() {
                total += crate::ccproxy::utils::token_estimator::estimate_tokens(reasoning);
            }
            total
        })
        .sum::<f64>()
        .round() as usize
}

fn restore_execution_context_from_manual_clear_marker(
    marker: Option<&WorkflowMessage>,
    fallback_execution_context: Option<&ExecutionContext>,
    session_id: &str,
    remaining_segment_id: i32,
    remaining_context_messages: &[WorkflowAiContextMessage],
) -> ExecutionContext {
    let metadata = marker.and_then(|message| message.metadata.as_ref());
    let mut restored_context = metadata
        .and_then(|value| value.get("previous_execution_context"))
        .cloned()
        .and_then(|value| serde_json::from_value::<ExecutionContext>(value).ok())
        .or_else(|| fallback_execution_context.cloned())
        .unwrap_or_else(|| ExecutionContext {
            session_id: session_id.to_string(),
            state: RuntimeState::Pending,
            wait_reason: None,
            current_segment_id: remaining_segment_id,
            current_step: 0,
            max_steps: 100,
            pending_tools: Vec::new(),
            last_action_summary: None,
            current_context_tokens: None,
            max_context_tokens: None,
            last_event_id: None,
            version: ExecutionContext::CURRENT_VERSION.to_string(),
            waiting_on_sub_agent_id: None,
            sub_agent_sessions: Vec::new(),
            pending_sub_agent_completions: Vec::new(),
            pending_final_review: None,
            pending_completion_reports: Vec::new(),
            removed_queued_user_message_ids: Vec::new(),
        });

    restored_context.session_id = session_id.to_string();
    restored_context.current_segment_id = metadata
        .and_then(|value| value.get("previous_segment_id"))
        .and_then(Value::as_i64)
        .map(|value| value as i32)
        .unwrap_or(remaining_segment_id);
    restored_context.current_context_tokens = Some(
        metadata
            .and_then(|value| value.get("previous_context_tokens"))
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or_else(|| estimate_ai_context_tokens(remaining_context_messages)),
    );
    restored_context.max_context_tokens = metadata
        .and_then(|value| value.get("previous_max_context_tokens"))
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .or(restored_context.max_context_tokens);

    restored_context
}

// =================================================
//  Structs
// =================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub id: Option<String>,
    #[serde(default)]
    pub is_automation_run: bool,
    pub parent_session_id: Option<String>,
    pub title: Option<String>,
    pub user_query: String,
    pub todo_list: Option<String>,
    #[serde(default = "default_workflow_status")]
    pub status: String,
    pub wait_reason: Option<String>,
    pub agent_id: String,
    pub agent_config: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

fn default_workflow_status() -> String {
    "pending".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowMessage {
    /// Durable workflow transcript history.
    /// This is the authoritative message record for audit, replay fallback,
    /// UI rendering, and semantic reporting.
    pub id: Option<i64>,
    pub session_id: String,
    pub role: String,
    pub message: String,
    pub reasoning: Option<String>,
    pub message_kind: String,
    pub message_subtype: Option<String>,
    pub segment_id: i32,
    pub source_event_type: Option<String>,
    pub metadata: Option<Value>,
    pub attached_context: Option<String>,
    pub step_type: Option<String>,
    pub step_index: i32,
    #[serde(default)]
    pub is_error: bool,
    pub error_type: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAiContextMessage {
    /// AI-only projected context cache.
    /// This is derived from `WorkflowMessage` using explicit projection rules
    /// and exists only to feed the LLM efficiently.
    /// It is rebuildable and must not be used as recovery, UI, or reporting authority.
    pub id: Option<i64>,
    pub session_id: String,
    pub segment_id: i32,
    pub role: String,
    pub message: String,
    pub reasoning: Option<String>,
    pub message_kind: String,
    pub message_subtype: Option<String>,
    pub metadata: Option<Value>,
    pub source_message_id: Option<i64>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSnapshot {
    /// Snapshot payload returned to commands/UI.
    /// Transcript authority comes from `messages`; runtime recovery authority
    /// comes separately from `workflow_snapshots` via `ExecutionContext`.
    pub workflow: Workflow,
    pub messages: Vec<WorkflowMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowMessageWindow {
    pub messages: Vec<WorkflowMessage>,
    pub before_message_id: Option<i64>,
    pub hidden_completed_task_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEfficiencyMetrics {
    pub total_tool_calls: u32,
    pub search_calls: u32,
    pub read_calls: u32,
    pub edit_calls: u32,
    pub verification_calls: u32,
    pub no_match_searches: u32,
    pub parallel_search_rounds: u32,
    pub parallel_read_rounds: u32,
    pub repeated_read_files: u32,
    pub repeated_read_events: u32,
    pub batch_edit_rounds: u32,
    pub pre_edit_read_coverage: u32,
    pub convergence_score: u32,
    pub execution_score: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEfficiencySessionReport {
    pub session_id: String,
    pub parent_session_id: Option<String>,
    pub title: Option<String>,
    pub user_query: String,
    pub status: String,
    pub metrics: WorkflowEfficiencyMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEfficiencyReport {
    pub root_session_id: String,
    pub main_agent: WorkflowEfficiencySessionReport,
    pub sub_agents: Vec<WorkflowEfficiencySessionReport>,
}

// =================================================
//  From Row Implementations
// =================================================

impl From<&Row<'_>> for Workflow {
    fn from(row: &Row<'_>) -> Self {
        Self {
            id: row.get("id").ok(),
            is_automation_run: row
                .get::<_, Option<i64>>("is_automation_run")
                .map(|value| value == Some(1))
                .unwrap_or(false),
            parent_session_id: row.get("parent_session_id").ok(),
            title: row.get("title").ok(),
            user_query: row.get("user_query").unwrap_or_default(),
            todo_list: row.get("todo_list").ok(),
            status: row.get("status").unwrap_or_else(|_| "pending".to_string()),
            wait_reason: row.get("wait_reason").ok(),
            agent_id: row.get("agent_id").unwrap_or_default(),
            agent_config: row.get("agent_config").ok(),
            created_at: row.get("created_at").ok(),
            updated_at: row.get("updated_at").ok(),
        }
    }
}

impl WorkflowMessage {
    pub(crate) fn normalize_classification(mut self) -> Self {
        if self.message_kind == "message"
            && self
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("type"))
                .and_then(Value::as_str)
                == Some("summary")
        {
            self.message_kind = "summary".to_string();
        }

        if self.message_subtype.is_none() {
            self.message_subtype = self
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("subtype"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string);
        }

        if self.role == "system" && self.message.trim() == "MANUAL_CLEAR_CONTEXT" {
            self.message_kind = "summary".to_string();
            self.message_subtype = Some("manual_clear_context".to_string());
            self.message.clear();
        }

        self
    }
}

impl From<&Row<'_>> for WorkflowMessage {
    fn from(row: &Row<'_>) -> Self {
        let metadata_str: Option<String> = row.get("metadata").ok();
        let metadata = metadata_str.and_then(|s| serde_json::from_str(&s).ok());

        Self {
            id: row.get("id").ok(),
            session_id: row.get("session_id").unwrap_or_default(),
            role: row.get("role").unwrap_or_default(),
            message: row.get("message").unwrap_or_default(),
            reasoning: row.get("reasoning").ok(),
            message_kind: row
                .get("message_kind")
                .unwrap_or_else(|_| "message".to_string()),
            message_subtype: row.get("message_subtype").ok(),
            segment_id: row.get("segment_id").unwrap_or(1),
            source_event_type: row.get("source_event_type").ok(),
            metadata,
            attached_context: row.get("attached_context").ok(),
            step_type: row.get("step_type").ok(),
            step_index: row.get("step_index").unwrap_or_default(),
            is_error: row
                .get::<_, Option<i32>>("is_error")
                .map(|v| v == Some(1))
                .unwrap_or(false),
            error_type: row.get("error_type").ok(),
            created_at: row.get("created_at").ok(),
        }
        .normalize_classification()
    }
}

impl From<&Row<'_>> for WorkflowEventRecord {
    fn from(row: &Row<'_>) -> Self {
        let event_data_str: String = row.get("event_data").unwrap_or_default();
        let event_data: Value = serde_json::from_str(&event_data_str).unwrap_or(Value::Null);

        Self {
            id: row.get("id").unwrap_or_default(),
            session_id: row.get("session_id").unwrap_or_default(),
            event_type: row.get("event_type").unwrap_or_default(),
            event_version: row.get("event_version").unwrap_or_default(),
            event_data,
            created_at: row.get("created_at").unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RewindPhase {
    Preserve,
    Planning,
    Implementation,
}

#[derive(Debug, Clone, PartialEq)]
struct TailRewindPlan {
    kind: &'static str,
    delete_message_boundary_id: Option<i64>,
    event_boundary_id: Option<i64>,
    phase: RewindPhase,
    message_metadata_updates: Vec<(i64, Value)>,
}

fn message_tool_name(message: &WorkflowMessage) -> Option<&str> {
    message
        .metadata
        .as_ref()
        .and_then(|meta| meta.get("tool_name"))
        .and_then(Value::as_str)
}

fn message_tool_call_id(message: &WorkflowMessage) -> Option<&str> {
    message
        .metadata
        .as_ref()
        .and_then(|meta| meta.get("tool_call_id"))
        .and_then(Value::as_str)
}

fn message_approval_status(message: &WorkflowMessage) -> Option<&str> {
    message
        .metadata
        .as_ref()
        .and_then(|meta| meta.get("approval_status"))
        .and_then(Value::as_str)
}

fn message_execution_status(message: &WorkflowMessage) -> Option<&str> {
    message
        .metadata
        .as_ref()
        .and_then(|meta| meta.get("execution_status"))
        .and_then(Value::as_str)
}

fn is_tool_observation_message(message: &WorkflowMessage) -> bool {
    message.role == "tool"
        && message.step_type.as_deref() == Some("observe")
        && message_tool_call_id(message).is_some()
}

fn is_pending_submit_plan_message(message: &WorkflowMessage) -> bool {
    message.role == "tool"
        && message_tool_name(message) == Some(crate::tools::TOOL_SUBMIT_PLAN)
        && message_approval_status(message) == Some("pending")
}

fn is_manual_clear_context_message(message: &WorkflowMessage) -> bool {
    message.role == "system"
        && message.message_kind == "summary"
        && message.message_subtype.as_deref() == Some("manual_clear_context")
}

fn is_user_input_wait_event(event: &WorkflowEventRecord) -> bool {
    event.event_type == "wait_entered"
        && event.event_data["wait_reason"].as_str() == Some("user_input")
}

fn is_approval_wait_event_for_tool(event: &WorkflowEventRecord, tool_call_id: &str) -> bool {
    if event.event_type != "wait_entered"
        || event.event_data["wait_reason"].as_str() != Some("approval")
    {
        return false;
    }

    event.event_data["pending_tools"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|tool| tool["tool_call_id"].as_str() == Some(tool_call_id))
}

fn approval_requested_event_id(events: &[WorkflowEventRecord], tool_call_id: &str) -> Option<i64> {
    events
        .iter()
        .rev()
        .find(|event| {
            event.event_type == "approval_requested"
                && event.event_data["tool_call_id"].as_str() == Some(tool_call_id)
        })
        .map(|event| event.id)
}

fn approval_resolved_event_id(events: &[WorkflowEventRecord], tool_call_id: &str) -> Option<i64> {
    events
        .iter()
        .rev()
        .find(|event| {
            event.event_type == "approval_resolved"
                && event.event_data["tool_call_id"].as_str() == Some(tool_call_id)
        })
        .map(|event| event.id)
}

fn tool_started_event_id(events: &[WorkflowEventRecord], tool_call_id: &str) -> Option<i64> {
    events
        .iter()
        .rev()
        .find(|event| {
            event.event_type == "tool_started"
                && event.event_data["tool_call_id"].as_str() == Some(tool_call_id)
        })
        .map(|event| event.id)
}

fn latest_approval_submitted_message<'a>(
    messages: &'a [WorkflowMessage],
    tool_call_id: &str,
) -> Option<&'a WorkflowMessage> {
    messages.iter().rev().find(|message| {
        is_tool_observation_message(message)
            && message_tool_call_id(message) == Some(tool_call_id)
            && message_execution_status(message) == Some("approval_submitted")
    })
}

fn latest_approved_plan_summary_message(messages: &[WorkflowMessage]) -> Option<&WorkflowMessage> {
    messages.iter().rev().find(|message| {
        message.id.is_some()
            && message.role == "system"
            && message.message_kind == "summary"
            && message.message_subtype.as_deref() == Some("approved_plan")
    })
}

fn pending_approval_event_boundary(
    events: &[WorkflowEventRecord],
    tool_call_id: &str,
) -> Option<i64> {
    let approval_requested_id = approval_requested_event_id(events, tool_call_id);
    let wait_entered_id = events
        .iter()
        .rev()
        .find(|event| is_approval_wait_event_for_tool(event, tool_call_id))
        .map(|event| event.id);

    match (wait_entered_id, approval_requested_id) {
        (Some(wait), Some(requested)) => Some(wait.min(requested)),
        (Some(wait), None) => Some(wait),
        (None, Some(requested)) => Some(requested),
        (None, None) => None,
    }
}

fn approval_tool_rewind_event_boundary(
    events: &[WorkflowEventRecord],
    tool_call_id: &str,
) -> Option<i64> {
    pending_approval_event_boundary(events, tool_call_id)
        .or_else(|| approval_resolved_event_id(events, tool_call_id))
        .or_else(|| tool_started_event_id(events, tool_call_id))
}

fn reverted_pending_approval_metadata(message: &WorkflowMessage) -> Option<Value> {
    let mut metadata = message.metadata.clone()?;
    metadata["approval_status"] = Value::String("pending".to_string());
    metadata["execution_status"] = Value::String("pending_approval".to_string());
    metadata["summary"] = Value::String(rust_i18n::t!("workflow.awaiting_approval").to_string());
    if let Some(object) = metadata.as_object_mut() {
        object.remove("hide_approval_details");
    }
    Some(metadata)
}

fn latest_user_input_wait_event_id(events: &[WorkflowEventRecord]) -> Option<i64> {
    let latest_wait = events
        .iter()
        .rev()
        .find(|event| is_user_input_wait_event(event))?;
    let resumed = events
        .iter()
        .rev()
        .any(|event| event.id > latest_wait.id && event.event_type == "user_input_received");
    if resumed {
        None
    } else {
        Some(latest_wait.id)
    }
}

fn latest_answered_user_input_wait_event_ids(events: &[WorkflowEventRecord]) -> Option<(i64, i64)> {
    let latest_wait = events
        .iter()
        .rev()
        .find(|event| is_user_input_wait_event(event))?;
    let latest_resume = events
        .iter()
        .rev()
        .find(|event| event.id > latest_wait.id && event.event_type == "user_input_received")?;
    Some((latest_wait.id, latest_resume.id))
}

fn latest_user_input_event_id(events: &[WorkflowEventRecord]) -> Option<i64> {
    events
        .iter()
        .rev()
        .find(|event| event.event_type == "user_input_received")
        .map(|event| event.id)
}

fn assistant_batch_message_id_for_tool_call(
    messages: &[WorkflowMessage],
    tool_call_id: &str,
) -> Option<i64> {
    messages
        .iter()
        .rev()
        .find(|message| {
            message.id.is_some()
                && message.role == "assistant"
                && message
                    .metadata
                    .as_ref()
                    .and_then(|metadata| metadata.get("tool_calls"))
                    .and_then(Value::as_array)
                    .is_some_and(|tool_calls| {
                        tool_calls.iter().any(|tool_call| {
                            tool_call
                                .get("id")
                                .and_then(Value::as_str)
                                .or_else(|| tool_call.get("tool_call_id").and_then(Value::as_str))
                                == Some(tool_call_id)
                        })
                    })
        })
        .and_then(|message| message.id)
}

fn determine_tail_rewind_plan(
    messages: &[WorkflowMessage],
    events: &[WorkflowEventRecord],
) -> Option<TailRewindPlan> {
    let latest_pending_submit_plan = messages
        .iter()
        .rev()
        .find(|message| message.id.is_some() && is_pending_submit_plan_message(message));

    if let Some((_, resume_event_id)) = latest_answered_user_input_wait_event_ids(events) {
        if let Some(user_message) = messages.iter().rev().find(|message| {
            message.id.is_some()
                && message.role == "user"
                && message.step_type.as_deref() != Some("observe")
                && message.metadata.is_none()
        }) {
            return Some(TailRewindPlan {
                kind: "ask_user_answered",
                delete_message_boundary_id: Some(user_message.id?),
                event_boundary_id: Some(resume_event_id),
                phase: RewindPhase::Preserve,
                message_metadata_updates: Vec::new(),
            });
        }
    }

    if let Some(wait_event_id) = latest_user_input_wait_event_id(events) {
        if let Some(ask_user_message) = messages.iter().rev().find(|message| {
            message.id.is_some()
                && message_tool_name(message) == Some(crate::tools::TOOL_ASK_USER)
                && message.step_type.as_deref() == Some("observe")
        }) {
            let delete_boundary_id = message_tool_call_id(ask_user_message)
                .and_then(|tool_call_id| {
                    assistant_batch_message_id_for_tool_call(messages, tool_call_id)
                })
                .or(ask_user_message.id);
            return Some(TailRewindPlan {
                kind: "ask_user_wait",
                delete_message_boundary_id: delete_boundary_id,
                event_boundary_id: Some(wait_event_id),
                phase: if latest_pending_submit_plan.is_some() {
                    RewindPhase::Planning
                } else {
                    RewindPhase::Implementation
                },
                message_metadata_updates: Vec::new(),
            });
        }
    }

    for message in messages.iter().rev().filter(|message| message.id.is_some()) {
        if is_manual_clear_context_message(message) {
            return Some(TailRewindPlan {
                kind: "manual_clear_context",
                delete_message_boundary_id: Some(message.id?),
                event_boundary_id: None,
                phase: RewindPhase::Preserve,
                message_metadata_updates: Vec::new(),
            });
        }

        if message.role == "user"
            && message.step_type.as_deref() != Some("observe")
            && message.metadata.is_none()
        {
            return Some(TailRewindPlan {
                kind: "user_message",
                delete_message_boundary_id: Some(message.id?),
                // Continuation messages append runtime events after a prior
                // terminal state. Trim those events together with the
                // message so replay restores the state shown by the tail.
                event_boundary_id: latest_user_input_event_id(events),
                phase: RewindPhase::Preserve,
                message_metadata_updates: Vec::new(),
            });
        }

        if !is_tool_observation_message(message) {
            continue;
        }

        let tool_call_id = match message_tool_call_id(message) {
            Some(id) => id,
            None => continue,
        };
        let tool_name = message_tool_name(message);
        let approval_status = message_approval_status(message);
        let execution_status = message_execution_status(message);

        if tool_name == Some(crate::tools::TOOL_ASK_USER) {
            continue;
        }

        if tool_name == Some(crate::tools::TOOL_SUBMIT_PLAN)
            && approval_status == Some("approved")
            && execution_status != Some("approval_submitted")
        {
            if let Some(event_boundary_id) = approval_resolved_event_id(events, tool_call_id) {
                let delete_boundary_id = latest_approved_plan_summary_message(messages)
                    .and_then(|message| message.id)
                    .unwrap_or(message.id?);
                return Some(TailRewindPlan {
                    kind: "approved_submit_plan",
                    delete_message_boundary_id: Some(delete_boundary_id),
                    event_boundary_id: Some(event_boundary_id),
                    phase: RewindPhase::Planning,
                    message_metadata_updates: Vec::new(),
                });
            }
        }

        if approval_status == Some("pending") {
            return Some(TailRewindPlan {
                kind: if tool_name == Some(crate::tools::TOOL_SUBMIT_PLAN) {
                    "pending_submit_plan"
                } else {
                    "pending_approval_tool"
                },
                delete_message_boundary_id: Some(message.id?),
                event_boundary_id: pending_approval_event_boundary(events, tool_call_id),
                phase: if tool_name == Some(crate::tools::TOOL_SUBMIT_PLAN) {
                    RewindPhase::Planning
                } else {
                    RewindPhase::Implementation
                },
                message_metadata_updates: Vec::new(),
            });
        }

        if approval_status == Some("approved") && execution_status == Some("approval_submitted") {
            let delete_boundary_id = if tool_name == Some(crate::tools::TOOL_SUBMIT_PLAN) {
                latest_approved_plan_summary_message(messages).and_then(|message| message.id)
            } else {
                latest_approval_submitted_message(messages, tool_call_id)
                    .and_then(|message| message.id)
                    .or(message.id)
            };
            return Some(TailRewindPlan {
                kind: if tool_name == Some(crate::tools::TOOL_SUBMIT_PLAN) {
                    "approved_submit_plan_waiting_execution"
                } else {
                    "approved_tool_waiting_execution"
                },
                delete_message_boundary_id: delete_boundary_id,
                event_boundary_id: if tool_name == Some(crate::tools::TOOL_SUBMIT_PLAN) {
                    approval_resolved_event_id(events, tool_call_id)
                } else {
                    approval_tool_rewind_event_boundary(events, tool_call_id)
                },
                phase: if tool_name == Some(crate::tools::TOOL_SUBMIT_PLAN) {
                    RewindPhase::Planning
                } else {
                    RewindPhase::Implementation
                },
                message_metadata_updates: if tool_name == Some(crate::tools::TOOL_SUBMIT_PLAN) {
                    reverted_pending_approval_metadata(message)
                        .and_then(|metadata| message.id.map(|id| (id, metadata)))
                        .into_iter()
                        .collect()
                } else {
                    Vec::new()
                },
            });
        }

        if approval_status == Some("approved")
            && tool_name != Some(crate::tools::TOOL_SUBMIT_PLAN)
            && message_execution_status(message) == Some("completed")
        {
            if let Some(event_boundary_id) =
                approval_tool_rewind_event_boundary(events, tool_call_id)
            {
                let delete_boundary_id = latest_approval_submitted_message(messages, tool_call_id)
                    .and_then(|submitted_message| submitted_message.id)
                    .unwrap_or(message.id?);
                return Some(TailRewindPlan {
                    kind: "approved_tool_completed",
                    delete_message_boundary_id: Some(delete_boundary_id),
                    event_boundary_id: Some(event_boundary_id),
                    phase: RewindPhase::Implementation,
                    message_metadata_updates: Vec::new(),
                });
            }
        }

        if let Some(event_boundary_id) = tool_started_event_id(events, tool_call_id) {
            return Some(TailRewindPlan {
                kind: if approval_status == Some("approved") {
                    if tool_name == Some(crate::tools::TOOL_SUBMIT_PLAN) {
                        "approved_submit_plan_completed"
                    } else {
                        "approved_tool_completed"
                    }
                } else {
                    "completed_tool"
                },
                delete_message_boundary_id: Some(message.id?),
                event_boundary_id: Some(event_boundary_id),
                phase: if tool_name == Some(crate::tools::TOOL_SUBMIT_PLAN) {
                    RewindPhase::Planning
                } else {
                    RewindPhase::Implementation
                },
                message_metadata_updates: Vec::new(),
            });
        }
    }

    None
}

fn prune_removed_tool_calls_from_assistant_message(
    message: &WorkflowMessage,
    removed_tool_call_ids: &HashSet<String>,
) -> Option<Option<Value>> {
    if message.role != "assistant" || removed_tool_call_ids.is_empty() {
        return None;
    }

    let mut metadata = message.metadata.clone()?;
    let tool_calls = metadata
        .get_mut("tool_calls")
        .and_then(|value| value.as_array_mut())?;
    let before_len = tool_calls.len();

    tool_calls.retain(|call| {
        let call_id = call
            .get("id")
            .and_then(|value| value.as_str())
            .or_else(|| call.get("tool_call_id").and_then(|value| value.as_str()));

        match call_id {
            Some(id) => !removed_tool_call_ids.contains(id),
            None => true,
        }
    });

    if tool_calls.len() == before_len {
        return None;
    }

    if tool_calls.is_empty() {
        // The assistant message and its tool calls are emitted as one interaction batch.
        // Once every tool call in that batch is rewound, the assistant batch itself
        // should disappear instead of leaving detached narration behind.
        return Some(None);
    }

    let has_text = !message.message.trim().is_empty()
        || message
            .reasoning
            .as_ref()
            .is_some_and(|reasoning| !reasoning.trim().is_empty());
    let has_tool_calls = metadata
        .get("tool_calls")
        .and_then(|value| value.as_array())
        .is_some_and(|calls| !calls.is_empty());

    if !has_text && !has_tool_calls {
        return Some(None);
    }

    Some(Some(metadata))
}

fn execution_context_to_workflow_status(context: &ExecutionContext) -> String {
    match context.state {
        RuntimeState::Pending => "pending".to_string(),
        RuntimeState::Running => "thinking".to_string(),
        RuntimeState::Stopping => "stopping".to_string(),
        RuntimeState::Waiting => match context.wait_reason {
            Some(WaitReason::Confirmation) => "paused".to_string(),
            Some(WaitReason::UserInput) => "awaiting_user".to_string(),
            Some(WaitReason::Approval) => "awaiting_approval".to_string(),
            Some(WaitReason::SubAgent) => "awaiting_sub_agent".to_string(),
            None => "pending".to_string(),
        },
        RuntimeState::Completed => "completed".to_string(),
        RuntimeState::Failed => "error".to_string(),
        RuntimeState::Cancelled => "cancelled".to_string(),
    }
}

fn sanitize_wait_reason_for_runtime_state(
    session_id: &str,
    state: &RuntimeState,
    wait_reason: &mut Option<WaitReason>,
) -> bool {
    if wait_reason.is_some() && *state != RuntimeState::Waiting {
        log::warn!(
            "[Workflow][session={}] snapshot.sanitize - clearing stale wait_reason={:?} for non-waiting state={:?}",
            session_id,
            wait_reason,
            state
        );
        *wait_reason = None;
        return true;
    }

    false
}

fn update_agent_config_phase(
    agent_config: Option<&str>,
    phase: RewindPhase,
) -> Result<String, StoreError> {
    if phase == RewindPhase::Preserve {
        if let Some(agent_config) = agent_config {
            if !agent_config.trim().is_empty() {
                return Ok(agent_config.to_string());
            }
        }
    }

    let mut config = agent_config
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));

    let phase_value = match phase {
        RewindPhase::Planning => Some("planning"),
        RewindPhase::Implementation => Some("implementation"),
        RewindPhase::Preserve => None,
    };

    if let Some(phase_value) = phase_value {
        config["phase"] = json!(phase_value);
    }

    serde_json::to_string(&config).map_err(StoreError::from)
}

impl From<&Row<'_>> for WorkflowAiContextMessage {
    fn from(row: &Row<'_>) -> Self {
        let metadata_str: Option<String> = row.get("metadata").ok();
        let metadata = metadata_str.and_then(|s| serde_json::from_str(&s).ok());

        Self {
            id: row.get("id").ok(),
            session_id: row.get("session_id").unwrap_or_default(),
            segment_id: row.get("segment_id").unwrap_or_default(),
            role: row.get("role").unwrap_or_default(),
            message: row.get("message").unwrap_or_default(),
            reasoning: row.get("reasoning").ok(),
            message_kind: row
                .get("message_kind")
                .unwrap_or_else(|_| "message".to_string()),
            message_subtype: row.get("message_subtype").ok(),
            metadata,
            source_message_id: row.get("source_message_id").ok(),
            created_at: row.get("created_at").ok(),
        }
    }
}

// =================================================
//  MainStore Implementation
// =================================================

impl MainStore {
    pub fn get_workflow_efficiency_report(
        &self,
        session_id: &str,
    ) -> Result<WorkflowEfficiencyReport, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let root_workflow: Workflow = conn.query_row(
            "SELECT * FROM workflows WHERE id = ?1",
            params![session_id],
            |row| Ok(Workflow::from(row)),
        )?;

        let mut stmt = conn.prepare(
            "WITH RECURSIVE workflow_tree AS (
                SELECT *
                FROM workflows
                WHERE id = ?1
                UNION ALL
                SELECT workflows.*
                FROM workflows
                JOIN workflow_tree ON workflows.parent_session_id = workflow_tree.id
            )
            SELECT *
            FROM workflow_tree
            ORDER BY CASE WHEN id = ?1 THEN 0 ELSE 1 END, created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| Ok(Workflow::from(row)))?;

        let mut reports = Vec::new();
        for row in rows {
            let workflow = row?;
            let messages = self.list_workflow_messages_for_session_locked(
                &conn,
                workflow.id.as_deref().unwrap_or_default(),
            )?;
            let metrics = compute_efficiency_metrics(&messages);
            reports.push(WorkflowEfficiencySessionReport {
                session_id: workflow.id.clone().unwrap_or_default(),
                parent_session_id: workflow.parent_session_id.clone(),
                title: workflow.title.clone(),
                user_query: workflow.user_query.clone(),
                status: workflow.status.clone(),
                metrics,
            });
        }

        let main_agent = reports
            .iter()
            .find(|report| report.session_id == session_id)
            .cloned()
            .unwrap_or_else(|| WorkflowEfficiencySessionReport {
                session_id: session_id.to_string(),
                parent_session_id: root_workflow.parent_session_id.clone(),
                title: root_workflow.title.clone(),
                user_query: root_workflow.user_query.clone(),
                status: root_workflow.status.clone(),
                metrics: WorkflowEfficiencyMetrics::default(),
            });

        let sub_agents = reports
            .into_iter()
            .filter(|report| report.session_id != session_id)
            .collect();

        Ok(WorkflowEfficiencyReport {
            root_session_id: session_id.to_string(),
            main_agent,
            sub_agents,
        })
    }

    fn list_workflow_messages_for_session_locked(
        &self,
        conn: &rusqlite::Connection,
        session_id: &str,
    ) -> Result<Vec<WorkflowMessage>, StoreError> {
        let mut stmt = conn.prepare(
            "SELECT *
             FROM workflow_messages
             WHERE session_id = ?1
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| Ok(WorkflowMessage::from(row)))?;
        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        Ok(messages)
    }

    pub fn create_workflow(
        &self,
        id: &str,
        user_query: &str,
        agent_id: &str,
        agent_config: Option<String>,
        parent_session_id: Option<&str>,
    ) -> Result<Workflow, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        conn.execute(
            "INSERT INTO workflows (id, parent_session_id, user_query, agent_id, agent_config, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, parent_session_id, user_query, agent_id, agent_config, "pending"],
        )?;

        let workflow: Workflow = conn.query_row(
            "SELECT * FROM workflows WHERE id = ?1",
            params![id],
            |row| Ok(Workflow::from(row)),
        )?;

        Ok(workflow)
    }

    pub fn list_workflows(&self) -> Result<Vec<Workflow>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT workflows.*,
                    EXISTS(
                        SELECT 1
                        FROM workflow_automation_runs
                        WHERE workflow_session_id = workflows.id
                    ) AS is_automation_run
             FROM workflows
             WHERE parent_session_id IS NULL
               AND id NOT LIKE 'subagent\\_%' ESCAPE '\\'
               AND id NOT LIKE 'task\\_%' ESCAPE '\\'
             ORDER BY updated_at DESC, created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| Ok(Workflow::from(row)))?;
        let mut workflows = Vec::new();
        for row in rows {
            workflows.push(row?);
        }
        Ok(workflows)
    }

    pub fn list_nonterminal_child_workflows(&self) -> Result<Vec<Workflow>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT * FROM workflows
             WHERE parent_session_id IS NOT NULL
               AND LOWER(status) NOT IN ('completed', 'error', 'failed', 'cancelled')
             ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], |row| Ok(Workflow::from(row)))?;
        let mut workflows = Vec::new();
        for row in rows {
            workflows.push(row?);
        }
        Ok(workflows)
    }

    pub fn delete_workflow(&self, id: &str) -> Result<(), StoreError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let tx = conn.transaction()?;
        let workflow_tree_cte = "WITH RECURSIVE workflow_tree(id, depth) AS (
            SELECT id, 0 FROM workflows WHERE id = ?1
            UNION ALL
            SELECT workflows.id, workflow_tree.depth + 1
            FROM workflows
            JOIN workflow_tree ON workflows.parent_session_id = workflow_tree.id
        )";
        let workflow_ids = {
            let mut stmt = tx.prepare(&format!(
                "{workflow_tree_cte} SELECT id FROM workflow_tree ORDER BY depth DESC"
            ))?;
            let rows = stmt.query_map(params![id], |row| row.get::<_, String>(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            ids
        };

        for table in [
            "workflow_context_messages",
            "workflow_messages",
            "workflow_snapshots",
        ] {
            tx.execute(
                &format!(
                    "{workflow_tree_cte} DELETE FROM {table} WHERE session_id IN (SELECT id FROM workflow_tree)"
                ),
                params![id],
            )?;
        }

        // workflow_events is an audit/secondary table. If it is corrupted (e.g.,
        // "database disk image is malformed"), do not block the workflow deletion.
        // Log the error and continue so the primary workflow data still gets cleaned up.
        if let Err(e) = tx.execute(
            &format!(
                "{workflow_tree_cte} DELETE FROM workflow_events WHERE session_id IN (SELECT id FROM workflow_tree)"
            ),
            params![id],
        ) {
            log::error!(
                "[Workflow][session={}] Failed to delete workflow events (non-fatal, continuing): {}",
                id,
                e
            );
        }

        // Delete child workflow rows before their parent to satisfy parent_session_id FK.
        for workflow_id in workflow_ids {
            tx.execute("DELETE FROM workflows WHERE id = ?1", params![workflow_id])?;
        }

        tx.commit()?;
        Ok(())
    }

    pub fn delete_last_message(&self, session_id: &str) -> Result<bool, StoreError> {
        let (agent_config, messages, events) = {
            let conn = self
                .conn
                .lock()
                .map_err(|e| StoreError::LockError(e.to_string()))?;

            let workflow_row: Option<(Option<String>,)> = conn
                .query_row(
                    "SELECT agent_config
                     FROM workflows
                     WHERE id = ?1",
                    params![session_id],
                    |row| Ok((row.get(0)?,)),
                )
                .optional()?;

            let Some((agent_config,)) = workflow_row else {
                return Ok(false);
            };

            let messages: Vec<WorkflowMessage> = {
                let mut message_stmt = conn.prepare(
                    "SELECT * FROM workflow_messages
                     WHERE session_id = ?1
                     ORDER BY id ASC",
                )?;
                let mapped = message_stmt
                    .query_map(params![session_id], |row| Ok(WorkflowMessage::from(row)))?
                    .collect::<Result<Vec<_>, _>>()?;
                mapped
            };

            if messages.is_empty() {
                return Ok(false);
            }

            let events: Vec<WorkflowEventRecord> = {
                let mut event_stmt = conn.prepare(
                    "SELECT id, session_id, event_type, event_version, event_data, created_at
                     FROM workflow_events
                     WHERE session_id = ?1
                     ORDER BY id ASC",
                )?;
                let mapped = event_stmt
                    .query_map(params![session_id], |row| {
                        Ok(WorkflowEventRecord::from(row))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                mapped
            };

            (agent_config, messages, events)
        };

        let Some(plan) = determine_tail_rewind_plan(&messages, &events) else {
            return Ok(false);
        };

        let remaining_segment_id = match plan.delete_message_boundary_id {
            Some(boundary_id) => messages
                .iter()
                .filter(|message| message.id.unwrap_or_default() < boundary_id)
                .map(|message| message.segment_id)
                .max()
                .unwrap_or(1),
            None => messages
                .iter()
                .map(|message| message.segment_id)
                .max()
                .unwrap_or(1),
        };
        let removed_tool_call_ids: HashSet<String> = messages
            .iter()
            .filter(|message| {
                plan.delete_message_boundary_id
                    .is_some_and(|boundary_id| message.id.unwrap_or_default() >= boundary_id)
            })
            .filter_map(message_tool_call_id)
            .map(ToOwned::to_owned)
            .collect();
        let remaining_events: Vec<WorkflowEventRecord> = events
            .iter()
            .filter(|event| {
                plan.event_boundary_id
                    .map_or(true, |boundary| event.id < boundary)
            })
            .cloned()
            .collect();

        let updated_agent_config = update_agent_config_phase(agent_config.as_deref(), plan.phase)?;
        let preserve_manual_clear_context = plan.kind == "manual_clear_context";
        let current_execution_context = if preserve_manual_clear_context {
            self.get_execution_context(session_id)?
        } else {
            None
        };
        let deleted_manual_clear_marker = if preserve_manual_clear_context {
            plan.delete_message_boundary_id.and_then(|boundary_id| {
                messages
                    .iter()
                    .find(|message| message.id == Some(boundary_id))
                    .cloned()
            })
        } else {
            None
        };
        let rebuilt_snapshot = if remaining_events.is_empty() {
            None
        } else {
            let mut rebuilt_context =
                replay_events_to_execution_context(session_id, &remaining_events)
                    .map_err(|error| StoreError::InvalidData(error.to_string()))?;
            rebuilt_context.current_segment_id = remaining_segment_id;

            let context_json = serde_json::to_string(&rebuilt_context)?;
            let state_str = rebuilt_context.state.to_string();
            let wait_reason_str = rebuilt_context
                .wait_reason
                .as_ref()
                .map(|wait_reason| wait_reason.to_string());
            let sub_agent_sessions_json =
                serde_json::to_string(&rebuilt_context.sub_agent_sessions)?;
            let workflow_status = execution_context_to_workflow_status(&rebuilt_context);

            Some((
                context_json,
                rebuilt_context.version,
                state_str,
                wait_reason_str,
                rebuilt_context.waiting_on_sub_agent_id.clone(),
                sub_agent_sessions_json,
                workflow_status,
            ))
        };

        let mut conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let tx = conn.transaction()?;

        if preserve_manual_clear_context {
            tx.execute(
                "DELETE FROM workflow_context_messages
                 WHERE session_id = ?1 AND segment_id > ?2",
                params![session_id, remaining_segment_id],
            )?;
        } else {
            tx.execute(
                "DELETE FROM workflow_context_messages WHERE session_id = ?1",
                params![session_id],
            )?;
        }

        if let Some(boundary_id) = plan.delete_message_boundary_id {
            tx.execute(
                "DELETE FROM workflow_messages
                 WHERE session_id = ?1 AND id >= ?2",
                params![session_id, boundary_id],
            )?;
        }

        for (message_id, metadata) in &plan.message_metadata_updates {
            let metadata_json = serde_json::to_string(metadata)?;
            tx.execute(
                "UPDATE workflow_messages SET metadata = ?2 WHERE id = ?1",
                params![message_id, metadata_json],
            )?;
        }

        if !removed_tool_call_ids.is_empty() {
            for assistant_message in messages.iter().filter(|message| {
                plan.delete_message_boundary_id.is_some_and(|boundary_id| {
                    message.id.unwrap_or_default() < boundary_id && message.role == "assistant"
                })
            }) {
                let Some(message_id) = assistant_message.id else {
                    continue;
                };

                match prune_removed_tool_calls_from_assistant_message(
                    assistant_message,
                    &removed_tool_call_ids,
                ) {
                    Some(Some(metadata)) => {
                        let metadata_json = serde_json::to_string(&metadata)?;
                        tx.execute(
                            "UPDATE workflow_messages SET metadata = ?2 WHERE id = ?1",
                            params![message_id, metadata_json],
                        )?;
                    }
                    Some(None) => {
                        tx.execute(
                            "DELETE FROM workflow_messages WHERE id = ?1",
                            params![message_id],
                        )?;
                    }
                    None => {}
                }
            }
        }

        if let Some(event_boundary_id) = plan.event_boundary_id {
            tx.execute(
                "DELETE FROM workflow_events
                 WHERE session_id = ?1 AND id >= ?2",
                params![session_id, event_boundary_id],
            )?;
        }

        if remaining_events.is_empty() {
            if preserve_manual_clear_context {
                let remaining_context_messages: Vec<WorkflowAiContextMessage> = {
                    let mut stmt = tx.prepare(
                        "SELECT * FROM workflow_context_messages
                         WHERE session_id = ?1 AND segment_id = ?2
                         ORDER BY id ASC",
                    )?;
                    let rows = stmt
                        .query_map(params![session_id, remaining_segment_id], |row| {
                            Ok(WorkflowAiContextMessage::from(row))
                        })?;
                    rows.collect::<Result<Vec<_>, _>>()?
                };
                let restored_context = restore_execution_context_from_manual_clear_marker(
                    deleted_manual_clear_marker.as_ref(),
                    current_execution_context.as_ref(),
                    session_id,
                    remaining_segment_id,
                    &remaining_context_messages,
                );
                let context_json = serde_json::to_string(&restored_context)?;
                tx.execute(
                    "INSERT OR REPLACE INTO workflow_snapshots
                     (session_id, context_json, version, state, wait_reason, waiting_on_sub_agent_id, sub_agent_sessions, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CURRENT_TIMESTAMP)",
                    params![
                        session_id,
                        context_json,
                        restored_context.version,
                        restored_context.state.to_string(),
                        restored_context
                            .wait_reason
                            .as_ref()
                            .map(WaitReason::to_string),
                        restored_context.waiting_on_sub_agent_id.clone(),
                        serde_json::to_string(&restored_context.sub_agent_sessions)?,
                    ],
                )?;
                tx.execute(
                    "UPDATE workflows
                     SET status = ?2,
                         agent_config = ?3,
                         updated_at = CURRENT_TIMESTAMP
                     WHERE id = ?1",
                    params![
                        session_id,
                        execution_context_to_workflow_status(&restored_context),
                        updated_agent_config,
                    ],
                )?;
                tx.commit()?;
                return Ok(true);
            } else {
                tx.execute(
                    "DELETE FROM workflow_snapshots WHERE session_id = ?1",
                    params![session_id],
                )?;
            }
            tx.execute(
                "UPDATE workflows
                 SET status = 'pending',
                     agent_config = ?2,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?1",
                params![session_id, updated_agent_config],
            )?;
            tx.commit()?;
            return Ok(true);
        }

        let Some((
            context_json,
            version,
            state_str,
            wait_reason_str,
            waiting_on_sub_agent_id,
            sub_agent_sessions_json,
            workflow_status,
        )) = rebuilt_snapshot
        else {
            return Ok(false);
        };

        tx.execute(
            "INSERT OR REPLACE INTO workflow_snapshots
             (session_id, context_json, version, state, wait_reason, waiting_on_sub_agent_id, sub_agent_sessions, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CURRENT_TIMESTAMP)",
            params![
                session_id,
                context_json,
                version,
                state_str,
                wait_reason_str,
                waiting_on_sub_agent_id,
                sub_agent_sessions_json,
            ],
        )?;

        tx.execute(
            "UPDATE workflows
             SET status = ?2,
                 agent_config = ?3,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![session_id, workflow_status, updated_agent_config],
        )?;

        tx.commit()?;
        Ok(true)
    }

    pub fn get_workflow_message_window(
        &self,
        session_id: &str,
        before_message_id: Option<i64>,
        initial_visible_group_count: usize,
    ) -> Result<WorkflowMessageWindow, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let mut stmt = conn.prepare(
            "SELECT * FROM workflow_messages
             WHERE session_id = ?1 AND (?2 IS NULL OR id < ?2)
             ORDER BY id DESC",
        )?;
        let message_rows = stmt.query_map(params![session_id, before_message_id], |row| {
            Ok(WorkflowMessage::from(row))
        })?;

        let is_completed_task_boundary = |message: &WorkflowMessage| {
            if message.role != "tool" || message.is_error {
                return false;
            }
            let Some(metadata) = message.metadata.as_ref() else {
                return false;
            };
            let tool_name = metadata
                .get("tool_name")
                .and_then(Value::as_str)
                .or_else(|| {
                    metadata
                        .get("tool_call")
                        .and_then(|tool_call| tool_call.get("name"))
                        .and_then(Value::as_str)
                })
                .or_else(|| {
                    metadata
                        .get("tool_call")
                        .and_then(|tool_call| tool_call.get("function"))
                        .and_then(|function| function.get("name"))
                        .and_then(Value::as_str)
                })
                .unwrap_or_default();
            if tool_name != "complete_workflow" {
                return false;
            }
            let execution_status = metadata
                .get("execution_status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let approval_status = metadata
                .get("approval_status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let review_display_state = metadata
                .get("review_display_state")
                .and_then(Value::as_str)
                .unwrap_or_default();

            approval_status != "rejected"
                && review_display_state != "final_review_rejected"
                && (execution_status.is_empty() || execution_status == "completed")
        };

        let mut messages_desc = Vec::new();
        let mut completion_count = 0usize;
        let mut target_completion_count = if before_message_id.is_some() {
            1
        } else {
            initial_visible_group_count.max(1)
        };

        for row in message_rows {
            let message = row?;
            if is_completed_task_boundary(&message) {
                if completion_count >= target_completion_count {
                    break;
                }
                if before_message_id.is_none() && completion_count == 0 && !messages_desc.is_empty()
                {
                    target_completion_count = initial_visible_group_count.saturating_sub(1).max(1);
                }
                completion_count += 1;
            }
            messages_desc.push(message);
        }

        messages_desc.reverse();
        let before_message_id = messages_desc.first().and_then(|message| message.id);
        let hidden_completed_task_count = if let Some(oldest_message_id) = before_message_id {
            let mut count_stmt = conn.prepare(
                "SELECT metadata, role, is_error FROM workflow_messages
                 WHERE session_id = ?1 AND id < ?2
                 ORDER BY id ASC",
            )?;
            let rows = count_stmt.query_map(params![session_id, oldest_message_id], |row| {
                let metadata_text: Option<String> = row.get(0)?;
                let role: String = row.get(1)?;
                let is_error: bool = row.get(2)?;
                Ok(WorkflowMessage {
                    id: None,
                    session_id: String::new(),
                    role,
                    message: String::new(),
                    reasoning: None,
                    message_kind: String::new(),
                    message_subtype: None,
                    segment_id: 0,
                    source_event_type: None,
                    metadata: metadata_text.and_then(|value| serde_json::from_str(&value).ok()),
                    attached_context: None,
                    step_type: None,
                    step_index: 0,
                    is_error,
                    error_type: None,
                    created_at: None,
                })
            })?;
            let mut count = 0usize;
            for row in rows {
                if is_completed_task_boundary(&row?) {
                    count += 1;
                }
            }
            count
        } else {
            0
        };

        Ok(WorkflowMessageWindow {
            messages: messages_desc,
            before_message_id,
            hidden_completed_task_count,
        })
    }

    pub fn get_workflow_for_ui(&self, id: &str) -> Result<Workflow, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let mut workflow: Workflow = conn.query_row(
            "SELECT workflows.*,
                    EXISTS(
                        SELECT 1
                        FROM workflow_automation_runs
                        WHERE workflow_session_id = workflows.id
                    ) AS is_automation_run
             FROM workflows
             WHERE workflows.id = ?1",
            params![id],
            |row| Ok(Workflow::from(row)),
        )?;
        let snapshot_state_and_wait_reason: Option<(Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT state, wait_reason FROM workflow_snapshots WHERE session_id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        workflow.wait_reason = snapshot_state_and_wait_reason.and_then(|(state, wait_reason)| {
            if state.as_deref() == Some("waiting") {
                wait_reason
            } else {
                None
            }
        });
        Ok(workflow)
    }

    pub fn get_workflow_snapshot(&self, id: &str) -> Result<WorkflowSnapshot, StoreError> {
        let workflow = self.get_workflow_for_ui(id)?;
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let mut stmt =
            conn.prepare("SELECT * FROM workflow_messages WHERE session_id = ?1 ORDER BY id ASC")?;
        let messages_iter = stmt.query_map(params![id], |row| Ok(WorkflowMessage::from(row)))?;
        let mut messages = Vec::new();
        for msg in messages_iter {
            messages.push(msg?);
        }

        Ok(WorkflowSnapshot { workflow, messages })
    }

    pub fn get_tail_rewind_kind(
        &self,
        session_id: &str,
    ) -> Result<Option<&'static str>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let mut message_stmt =
            conn.prepare("SELECT * FROM workflow_messages WHERE session_id = ?1 ORDER BY id ASC")?;
        let message_rows =
            message_stmt.query_map(params![session_id], |row| Ok(WorkflowMessage::from(row)))?;
        let mut messages = Vec::new();
        for row in message_rows {
            messages.push(row?);
        }

        let mut event_stmt =
            conn.prepare("SELECT * FROM workflow_events WHERE session_id = ?1 ORDER BY id ASC")?;
        let event_rows = event_stmt.query_map(params![session_id], |row| {
            Ok(WorkflowEventRecord::from(row))
        })?;
        let mut events = Vec::new();
        for row in event_rows {
            events.push(row?);
        }

        Ok(determine_tail_rewind_plan(&messages, &events).map(|plan| plan.kind))
    }

    pub fn get_workflow(&self, id: &str) -> Result<Option<Workflow>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        conn.query_row(
            "SELECT * FROM workflows WHERE id = ?1",
            params![id],
            |row| Ok(Workflow::from(row)),
        )
        .optional()
        .map_err(StoreError::from)
    }

    pub fn add_workflow_message(
        &self,
        msg: &WorkflowMessage,
    ) -> Result<WorkflowMessage, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let metadata_json = msg
            .metadata
            .as_ref()
            .map(|m| serde_json::to_string(m).unwrap_or_default());

        conn.execute(
            "INSERT INTO workflow_messages (session_id, role, message, reasoning, message_kind, message_subtype, segment_id, source_event_type, metadata, attached_context, step_type, step_index, is_error, error_type)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                msg.session_id,
                msg.role,
                msg.message,
                msg.reasoning,
                msg.message_kind,
                msg.message_subtype,
                msg.segment_id,
                msg.source_event_type,
                metadata_json,
                msg.attached_context,
                msg.step_type,
                msg.step_index,
                if msg.is_error { 1 } else { 0 },
                msg.error_type,
            ],
        )?;

        conn.execute(
            "UPDATE workflows SET updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
            params![msg.session_id],
        )?;

        let id = conn.last_insert_rowid();
        let mut new_msg = msg.clone();
        new_msg.id = Some(id);
        Ok(new_msg)
    }

    pub fn update_workflow_message_metadata(
        &self,
        message_id: i64,
        metadata: &Value,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let metadata_json = serde_json::to_string(metadata).unwrap_or_default();

        conn.execute(
            "UPDATE workflow_messages SET metadata = ?1 WHERE id = ?2",
            params![metadata_json, message_id],
        )?;

        Ok(())
    }

    pub fn add_workflow_ai_context_message(
        &self,
        msg: &WorkflowAiContextMessage,
    ) -> Result<WorkflowAiContextMessage, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let metadata_json = msg
            .metadata
            .as_ref()
            .map(|m| serde_json::to_string(m).unwrap_or_default());

        conn.execute(
            "INSERT INTO workflow_context_messages (session_id, segment_id, role, message, reasoning, message_kind, message_subtype, metadata, source_message_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                msg.session_id,
                msg.segment_id,
                msg.role,
                msg.message,
                msg.reasoning,
                msg.message_kind,
                msg.message_subtype,
                metadata_json,
                msg.source_message_id,
            ],
        )?;

        let id = conn.last_insert_rowid();
        let mut new_msg = msg.clone();
        new_msg.id = Some(id);
        Ok(new_msg)
    }

    pub fn delete_workflow_ai_context_segment(
        &self,
        session_id: &str,
        segment_id: i32,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        conn.execute(
            "DELETE FROM workflow_context_messages WHERE session_id = ?1 AND segment_id = ?2",
            params![session_id, segment_id],
        )?;

        Ok(())
    }

    pub fn update_workflow_status(&self, id: &str, status: &str) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflows
             SET status = ?1,
                 updated_at = CASE
                     WHEN status IS NOT ?1 THEN CURRENT_TIMESTAMP
                     ELSE updated_at
                 END
             WHERE id = ?2",
            params![status, id],
        )?;
        Ok(())
    }

    pub fn update_workflow_title(&self, id: &str, title: &str) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflows
             SET title = ?1,
                 updated_at = CASE
                     WHEN title IS NOT ?1 THEN CURRENT_TIMESTAMP
                     ELSE updated_at
                 END
             WHERE id = ?2",
            params![title, id],
        )?;
        Ok(())
    }

    pub fn update_workflow_title_and_query(
        &self,
        id: &str,
        title: &str,
        user_query: &str,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflows
             SET title = ?1,
                 user_query = ?2,
                 updated_at = CASE
                     WHEN title IS NOT ?1 OR user_query IS NOT ?2 THEN CURRENT_TIMESTAMP
                     ELSE updated_at
                 END
             WHERE id = ?3",
            params![title, user_query, id],
        )?;
        Ok(())
    }

    pub fn update_workflow_query(&self, id: &str, user_query: &str) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflows
             SET user_query = ?1,
                 updated_at = CASE
                     WHEN user_query IS NOT ?1 THEN CURRENT_TIMESTAMP
                     ELSE updated_at
                 END
             WHERE id = ?2",
            params![user_query, id],
        )?;
        Ok(())
    }

    pub fn update_workflow_todo_list(&self, id: &str, todo_list: &str) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflows
             SET todo_list = ?1,
                 updated_at = CASE
                     WHEN todo_list IS NOT ?1 THEN CURRENT_TIMESTAMP
                     ELSE updated_at
                 END
             WHERE id = ?2",
            params![todo_list, id],
        )?;
        Ok(())
    }

    pub fn update_workflow_agent_config(
        &self,
        id: &str,
        agent_config: &str,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflows
             SET agent_config = ?1,
                 updated_at = CASE
                     WHEN agent_config IS NOT ?1 THEN CURRENT_TIMESTAMP
                     ELSE updated_at
                 END
             WHERE id = ?2",
            params![agent_config, id],
        )?;
        Ok(())
    }

    pub fn update_workflow_agent_id(&self, id: &str, agent_id: &str) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflows
             SET agent_id = ?1,
                 updated_at = CASE
                     WHEN agent_id IS NOT ?1 THEN CURRENT_TIMESTAMP
                     ELSE updated_at
                 END
             WHERE id = ?2",
            params![agent_id, id],
        )?;
        Ok(())
    }

    pub fn get_todo_list_for_workflow(&self, id: &str) -> Result<Vec<Value>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let todo_list_str: Option<String> = conn
            .query_row(
                "SELECT todo_list FROM workflows WHERE id = ?1",
                params![id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();

        Ok(todo_list_str
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default())
    }

    // ExecutionContext Snapshot Operations

    pub fn get_execution_context(
        &self,
        session_id: &str,
    ) -> Result<Option<ExecutionContext>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let result: Option<String> = conn
            .query_row(
                "SELECT context_json FROM workflow_snapshots WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?;

        match result {
            Some(context_json) => {
                let mut ctx: ExecutionContext = serde_json::from_str(&context_json)?;
                let _ = sanitize_wait_reason_for_runtime_state(
                    session_id,
                    &ctx.state,
                    &mut ctx.wait_reason,
                );
                log::info!(
                    "[Workflow][session={}] snapshot.read - state={:?}, wait_reason={:?}, pending_tools={}",
                    session_id,
                    ctx.state,
                    ctx.wait_reason,
                    ctx.pending_tools.len()
                );
                Ok(Some(ctx))
            }
            None => Ok(None),
        }
    }

    pub fn upsert_execution_context(&self, ctx: &ExecutionContext) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let context_json = serde_json::to_string(ctx)?;
        let state_str = ctx.state.to_string();
        let wait_reason_str = ctx.wait_reason.as_ref().map(|wr| wr.to_string());
        let sub_agent_sessions_json = serde_json::to_string(&ctx.sub_agent_sessions)?;

        conn.execute(
            "INSERT OR REPLACE INTO workflow_snapshots
             (session_id, context_json, version, state, wait_reason, waiting_on_sub_agent_id, sub_agent_sessions, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CURRENT_TIMESTAMP)",
            params![
                ctx.session_id,
                context_json,
                ctx.version,
                state_str,
                wait_reason_str,
                ctx.waiting_on_sub_agent_id.clone(),
                sub_agent_sessions_json,
            ],
        )?;

        log::info!(
            "[Workflow][session={}] snapshot.write - state={:?}, wait_reason={:?}, pending_tools={}",
            ctx.session_id,
            ctx.state,
            ctx.wait_reason,
            ctx.pending_tools.len()
        );

        Ok(())
    }

    // Workflow Event Operations

    pub fn append_workflow_event(&self, event: &WorkflowEvent) -> Result<i64, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let event_type_str = event.event_type.as_str().to_string();
        let event_data_str = serde_json::to_string(&event.event_data)?;

        conn.execute(
            "INSERT INTO workflow_events (session_id, event_type, event_version, event_data)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                event.session_id,
                event_type_str,
                event.version,
                event_data_str
            ],
        )?;

        let event_id = conn.last_insert_rowid();

        log::info!(
            "[Workflow][session={}] event.append - type={:?}, event_id={}",
            event.session_id,
            event.event_type,
            event_id
        );

        Ok(event_id)
    }

    pub fn list_workflow_events(
        &self,
        session_id: &str,
    ) -> Result<Vec<WorkflowEventRecord>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let mut stmt = conn.prepare(
            "SELECT id, session_id, event_type, event_version, event_data, created_at
             FROM workflow_events
             WHERE session_id = ?1
             ORDER BY id ASC",
        )?;

        let rows = stmt.query_map(params![session_id], |row| {
            Ok(WorkflowEventRecord::from(row))
        })?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }

        log::info!(
            "[Workflow][session={}] event.list - count={}",
            session_id,
            events.len()
        );

        Ok(events)
    }

    pub fn get_last_event_id(&self, session_id: &str) -> Result<Option<i64>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let result: Option<i64> = conn.query_row(
            "SELECT MAX(id) FROM workflow_events WHERE session_id = ?1",
            params![session_id],
            |row| row.get::<_, Option<i64>>(0),
        )?;

        Ok(result)
    }
}

fn compute_efficiency_metrics(messages: &[WorkflowMessage]) -> WorkflowEfficiencyMetrics {
    let mut metrics = WorkflowEfficiencyMetrics::default();
    let mut read_counts: HashMap<String, u32> = HashMap::new();
    let mut seen_read_files: HashSet<String> = HashSet::new();
    let mut edited_files: HashSet<String> = HashSet::new();
    let mut read_before_edit_files: HashSet<String> = HashSet::new();

    for message in messages {
        if message.role == "assistant" {
            let tool_calls = extract_tool_calls_from_metadata(message.metadata.as_ref());
            let search_count = tool_calls
                .iter()
                .filter(|tool_call| is_search_tool(&tool_call.name))
                .count();
            let read_count = tool_calls
                .iter()
                .filter(|tool_call| is_read_tool(&tool_call.name))
                .count();
            let edit_count = tool_calls
                .iter()
                .filter(|tool_call| is_edit_tool(&tool_call.name))
                .count();

            if search_count >= 2 {
                metrics.parallel_search_rounds += 1;
            }
            if read_count >= 2 {
                metrics.parallel_read_rounds += 1;
            }
            if edit_count >= 2 {
                metrics.batch_edit_rounds += 1;
            }
            continue;
        }

        if message.role != "tool" {
            continue;
        }

        let tool_name = extract_tool_name(message.metadata.as_ref());
        if matches!(
            tool_name.as_deref(),
            Some("complete_workflow" | "answer_user")
        ) {
            continue;
        }

        metrics.total_tool_calls += 1;

        if let Some(tool_name) = tool_name.as_deref() {
            if is_search_tool(tool_name) {
                metrics.search_calls += 1;
                if message.message.contains("[No matches found]") {
                    metrics.no_match_searches += 1;
                }
            }

            if is_read_tool(tool_name) {
                metrics.read_calls += 1;
                for path in extract_paths_for_tool(message, tool_name) {
                    let count = read_counts.entry(path.clone()).or_insert(0);
                    *count += 1;
                    seen_read_files.insert(path);
                }
            }

            if is_edit_tool(tool_name) {
                metrics.edit_calls += 1;
                for path in extract_paths_for_tool(message, tool_name) {
                    if seen_read_files.contains(&path) {
                        read_before_edit_files.insert(path.clone());
                    }
                    edited_files.insert(path);
                }
            }

            if is_verification_tool(tool_name, message.metadata.as_ref()) {
                metrics.verification_calls += 1;
            }
        }
    }

    metrics.repeated_read_files = read_counts.values().filter(|count| **count > 1).count() as u32;
    metrics.repeated_read_events = read_counts
        .values()
        .map(|count| count.saturating_sub(1))
        .sum();

    metrics.pre_edit_read_coverage = if edited_files.is_empty() {
        100
    } else {
        ((read_before_edit_files.len() as f64 / edited_files.len() as f64) * 100.0).round() as u32
    };

    metrics.convergence_score = score_convergence(&metrics);
    metrics.execution_score = score_execution(&metrics);

    metrics
}

#[derive(Debug, Clone)]
struct ToolCallShape {
    name: String,
}

fn extract_tool_calls_from_metadata(metadata: Option<&Value>) -> Vec<ToolCallShape> {
    metadata
        .and_then(|meta| meta.get("tool_calls"))
        .and_then(|tool_calls| tool_calls.as_array())
        .map(|tool_calls| {
            tool_calls
                .iter()
                .filter_map(|tool_call| {
                    tool_call
                        .get("function")
                        .and_then(|function| function.get("name"))
                        .and_then(|name| name.as_str())
                        .map(|name| ToolCallShape {
                            name: name.to_string(),
                        })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_tool_name(metadata: Option<&Value>) -> Option<String> {
    metadata
        .and_then(|meta| meta.get("tool_name"))
        .and_then(|tool_name| tool_name.as_str())
        .map(str::to_string)
        .or_else(|| {
            metadata
                .and_then(|meta| meta.get("tool_call"))
                .and_then(|tool_call| tool_call.get("function"))
                .and_then(|function| function.get("name"))
                .and_then(|name| name.as_str())
                .map(str::to_string)
        })
}

fn is_search_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "glob" | "grep" | "list_dir" | "search_workspace_files" | "web_search"
    )
}

fn is_read_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "read_file" | "read_git_base_text_file" | "web_fetch"
    )
}

fn is_edit_tool(tool_name: &str) -> bool {
    matches!(tool_name, "edit_file" | "write_file")
}

fn is_verification_tool(tool_name: &str, metadata: Option<&Value>) -> bool {
    if !matches!(tool_name, "bash" | "execute_command") {
        return false;
    }

    let Some(command_text) = metadata
        .and_then(|meta| meta.get("tool_call"))
        .and_then(|tool_call| tool_call.get("function"))
        .and_then(|function| function.get("arguments"))
        .and_then(|arguments| arguments.as_str())
    else {
        return false;
    };

    let command_text = command_text.to_ascii_lowercase();
    [
        "cargo check",
        "cargo test",
        "cargo clippy",
        "npm test",
        "pnpm test",
        "pnpm lint",
        "pnpm build",
        "go test",
        "pytest",
        "vitest",
        "jest",
        "ruff check",
    ]
    .iter()
    .any(|needle| command_text.contains(needle))
}

fn extract_paths_for_tool(message: &WorkflowMessage, tool_name: &str) -> Vec<String> {
    let mut paths = Vec::new();

    if matches!(tool_name, "read_file" | "read_git_base_text_file") {
        if let Some(path) = extract_file_content_path(&message.message) {
            paths.push(path);
        }
    }

    if let Some(arguments) = message
        .metadata
        .as_ref()
        .and_then(|meta| meta.get("tool_call"))
        .and_then(|tool_call| tool_call.get("function"))
        .and_then(|function| function.get("arguments"))
        .and_then(|arguments| arguments.as_str())
        .and_then(|arguments| serde_json::from_str::<Value>(arguments).ok())
    {
        collect_paths_from_json(&arguments, &mut paths);
    }

    paths.sort();
    paths.dedup();
    paths
}

fn extract_file_content_path(message: &str) -> Option<String> {
    let marker = "<file_content path=\"";
    let start = message.find(marker)? + marker.len();
    let rest = &message[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn collect_paths_from_json(value: &Value, output: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                let lower = key.to_ascii_lowercase();
                let should_collect = matches!(
                    lower.as_str(),
                    "file_path" | "path" | "relative_path" | "target_file"
                );
                if should_collect {
                    if let Some(path) = inner.as_str() {
                        output.push(path.to_string());
                    }
                }
                collect_paths_from_json(inner, output);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_paths_from_json(item, output);
            }
        }
        _ => {}
    }
}

fn score_convergence(metrics: &WorkflowEfficiencyMetrics) -> u32 {
    let mut score: i32 = 72;
    score += ((metrics.parallel_search_rounds.min(2)) as i32) * 4;
    score += ((metrics.parallel_read_rounds.min(2)) as i32) * 3;
    score -= ((metrics.no_match_searches.min(3)) as i32) * 5;
    score -= ((metrics.repeated_read_events.min(8)) as i32) * 2;

    if metrics.edit_calls > 0 || metrics.verification_calls > 0 {
        score += 6;
    }

    if metrics.total_tool_calls > 0 && metrics.search_calls == 0 && metrics.read_calls > 0 {
        score += 2;
    }

    score.clamp(35, 95) as u32
}

fn score_execution(metrics: &WorkflowEfficiencyMetrics) -> u32 {
    let mut score: i32 = 72;

    if metrics.edit_calls > 0 {
        score += ((metrics.pre_edit_read_coverage as i32) * 12) / 100;
        score += ((metrics.batch_edit_rounds.min(2)) as i32) * 3;
        if metrics.pre_edit_read_coverage < 50 {
            score -= 8;
        }
    }

    if metrics.verification_calls > 0 {
        score += 10;
    } else if metrics.edit_calls > 0 {
        score -= 10;
    }

    if metrics.edit_calls == 0 && metrics.verification_calls == 0 {
        score += 2;
    }

    score.clamp(45, 96) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::types::{PendingTool, RuntimeState, WaitReason};
    use tempfile::tempdir;

    fn create_test_store() -> MainStore {
        let dir = tempdir().expect("failed to create temp dir");
        let db_path = dir.path().join("workflow_phase4_test.db");
        MainStore::new(db_path).expect("failed to create MainStore")
    }

    #[test]
    fn get_execution_context_clears_stale_wait_reason_for_non_waiting_state() {
        let store = create_test_store();
        let session_id = "snapshot-stale-wait-reason";
        seed_agent(&store, "agent-test");
        store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");

        let mut context = ExecutionContext::new(session_id.to_string());
        context.state = RuntimeState::Running;
        context.wait_reason = Some(WaitReason::UserInput);
        store
            .upsert_execution_context(&context)
            .expect("failed to persist execution context");

        let restored = store
            .get_execution_context(session_id)
            .expect("failed to load execution context")
            .expect("expected execution context");

        assert_eq!(restored.state, RuntimeState::Running);
        assert_eq!(restored.wait_reason, None);
    }

    #[test]
    fn get_workflow_for_ui_ignores_stale_wait_reason_for_non_waiting_snapshot() {
        let store = create_test_store();
        let session_id = "workflow-ui-stale-wait-reason";
        seed_agent(&store, "agent-test");
        store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");

        let mut context = ExecutionContext::new(session_id.to_string());
        context.state = RuntimeState::Running;
        context.wait_reason = Some(WaitReason::UserInput);
        store
            .upsert_execution_context(&context)
            .expect("failed to persist execution context");

        let workflow = store
            .get_workflow_for_ui(session_id)
            .expect("failed to load workflow");

        assert_eq!(workflow.wait_reason, None);
    }

    fn seed_agent(store: &MainStore, agent_id: &str) {
        let conn = store
            .conn
            .lock()
            .expect("failed to lock db connection for agent seed");
        conn.execute(
            "INSERT INTO agents (id, name, system_prompt, agent_type, max_contexts)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                agent_id,
                format!("Agent Test {}", agent_id),
                "You are a test agent.",
                "autonomous",
                20
            ],
        )
        .expect("failed to seed agent");
    }

    #[test]
    fn approval_recovery_authority_comes_from_execution_context_not_transcript() {
        let store = create_test_store();
        let session_id = "approval-recovery-authority";
        seed_agent(&store, "agent-test");
        store
            .create_workflow(
                session_id,
                "Inspect approval recovery",
                "agent-test",
                None,
                None,
            )
            .expect("failed to create workflow");

        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "assistant".to_string(),
                message: "I'll inspect the workflow state.".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_calls": [
                        {
                            "id": "complete_1",
                            "type": "function",
                            "function": {
                                "name": "complete_workflow",
                                "arguments": { "summary": "Done" }
                            }
                        }
                    ]
                })),
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add assistant message");

        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "tool".to_string(),
                message: "{\"summary\":\"Done\"}".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_call_id": "complete_1",
                    "tool_name": "complete_workflow",
                    "execution_status": "completed",
                    "summary": "Done"
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add completion tool message");

        let mut context = ExecutionContext::new(session_id.to_string());
        context.state = RuntimeState::Waiting;
        context.wait_reason = Some(WaitReason::Approval);
        context.pending_tools = vec![PendingTool {
            tool_call_id: "tool_571ae521".to_string(),
            tool_name: "bash".to_string(),
            arguments: json!({ "command": "sqlite3 workflow.db" }),
            details: Some(json!({
                "command": "sqlite3 workflow.db",
                "description": "Inspect workflow state"
            })),
            display_type: Some("text".to_string()),
        }];
        store
            .upsert_execution_context(&context)
            .expect("failed to persist approval snapshot");
        store
            .update_workflow_status(session_id, "awaiting_approval")
            .expect("failed to update workflow status");

        let workflow = store
            .get_workflow_for_ui(session_id)
            .expect("failed to load workflow for ui");
        assert_eq!(workflow.status, "awaiting_approval");
        assert_eq!(workflow.wait_reason.as_deref(), Some("approval"));

        let snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to load workflow snapshot");
        assert_eq!(snapshot.messages.len(), 2);
        assert!(
            !snapshot.messages.iter().any(|message| {
                message.role == "tool"
                    && message
                        .metadata
                        .as_ref()
                        .and_then(|meta| meta.get("tool_call_id"))
                        .and_then(Value::as_str)
                        == Some("tool_571ae521")
            }),
            "transcript should not need a pending tool observation for approval recovery"
        );

        let restored = store
            .get_execution_context(session_id)
            .expect("failed to load execution context")
            .expect("approval snapshot should exist");
        assert_eq!(restored.state, RuntimeState::Waiting);
        assert_eq!(restored.wait_reason, Some(WaitReason::Approval));
        assert_eq!(restored.pending_tools.len(), 1);
        assert_eq!(restored.pending_tools[0].tool_call_id, "tool_571ae521");
        assert_eq!(restored.pending_tools[0].tool_name, "bash");
        assert_eq!(
            restored.pending_tools[0].arguments,
            json!({ "command": "sqlite3 workflow.db" })
        );
        assert_eq!(
            restored.pending_tools[0].details,
            Some(json!({
                "command": "sqlite3 workflow.db",
                "description": "Inspect workflow state"
            }))
        );
        assert_eq!(
            restored.pending_tools[0].display_type.as_deref(),
            Some("text")
        );
    }

    fn add_window_test_message(
        store: &MainStore,
        session_id: &str,
        message: &str,
        completed: bool,
    ) -> WorkflowMessage {
        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: if completed { "tool" } else { "user" }.to_string(),
                message: message.to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: completed.then(|| {
                    json!({
                        "tool_name": "complete_workflow",
                        "execution_status": "completed"
                    })
                }),
                attached_context: None,
                step_type: None,
                step_index: 0,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add window test message")
    }

    #[test]
    fn test_workflow_message_window_loads_recent_and_earlier_complete_tasks() {
        let store = create_test_store();
        seed_agent(&store, "agent-window");
        store
            .create_workflow("window-session", "Window query", "agent-window", None, None)
            .expect("failed to create workflow");

        for task in 1..=3 {
            add_window_test_message(
                &store,
                "window-session",
                &format!("task-{task}-input"),
                false,
            );
            add_window_test_message(
                &store,
                "window-session",
                &format!("task-{task}-completed"),
                true,
            );
        }
        add_window_test_message(&store, "window-session", "active-task", false);

        let recent = store
            .get_workflow_message_window("window-session", None, 2)
            .expect("failed to load recent window");
        assert_eq!(
            recent
                .messages
                .iter()
                .map(|message| message.message.as_str())
                .collect::<Vec<_>>(),
            vec!["task-3-input", "task-3-completed", "active-task"]
        );
        assert_eq!(recent.hidden_completed_task_count, 2);

        let earlier = store
            .get_workflow_message_window("window-session", recent.before_message_id, 2)
            .expect("failed to load earlier window");
        assert_eq!(
            earlier
                .messages
                .iter()
                .map(|message| message.message.as_str())
                .collect::<Vec<_>>(),
            vec!["task-2-input", "task-2-completed"]
        );
        assert_eq!(earlier.hidden_completed_task_count, 1);

        let oldest = store
            .get_workflow_message_window("window-session", earlier.before_message_id, 1)
            .expect("failed to load oldest window");
        assert_eq!(
            oldest
                .messages
                .iter()
                .map(|message| message.message.as_str())
                .collect::<Vec<_>>(),
            vec!["task-1-input", "task-1-completed"]
        );
        assert_eq!(oldest.hidden_completed_task_count, 0);

        let loaded_ids = recent
            .messages
            .iter()
            .chain(earlier.messages.iter())
            .chain(oldest.messages.iter())
            .filter_map(|message| message.id)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(loaded_ids.len(), 7, "paged windows must not overlap");

        store
            .create_workflow(
                "completed-window",
                "Completed query",
                "agent-window",
                None,
                None,
            )
            .expect("failed to create completed workflow");
        for task in 1..=2 {
            add_window_test_message(
                &store,
                "completed-window",
                &format!("completed-{task}-input"),
                false,
            );
            add_window_test_message(
                &store,
                "completed-window",
                &format!("completed-{task}-done"),
                true,
            );
        }
        let completed_window = store
            .get_workflow_message_window("completed-window", None, 2)
            .expect("failed to load completed-only window");
        assert_eq!(completed_window.messages.len(), 4);
        assert_eq!(completed_window.hidden_completed_task_count, 0);
    }

    #[test]
    fn test_normalize_legacy_message_classification_from_metadata() {
        let message = WorkflowMessage {
            id: Some(1),
            session_id: "legacy-session".to_string(),
            role: "system".to_string(),
            message: String::new(),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: None,
            metadata: Some(serde_json::json!({
                "type": "summary",
                "subtype": "manual_clear_context"
            })),
            attached_context: None,
            step_type: None,
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        }
        .normalize_classification();

        assert_eq!(message.message_kind, "summary");
        assert_eq!(
            message.message_subtype.as_deref(),
            Some("manual_clear_context")
        );
    }

    #[test]
    fn test_snapshot_hydrates_legacy_manual_clear_context_text() {
        let store = create_test_store();
        seed_agent(&store, "agent-main");
        store
            .create_workflow("legacy-session", "Legacy query", "agent-main", None, None)
            .expect("failed to create legacy workflow");

        let conn = store
            .conn
            .lock()
            .expect("failed to lock db connection for legacy message seed");
        conn.execute(
            "INSERT INTO workflow_messages
             (session_id, role, message, message_kind, segment_id, step_index, is_error)
             VALUES (?1, 'system', 'MANUAL_CLEAR_CONTEXT', 'message', 1, 0, 0)",
            params!["legacy-session"],
        )
        .expect("failed to insert legacy manual clear-context message");
        drop(conn);

        let snapshot = store
            .get_workflow_snapshot("legacy-session")
            .expect("failed to hydrate legacy workflow snapshot");
        let message = snapshot
            .messages
            .first()
            .expect("legacy message should be present");

        assert_eq!(message.message_kind, "summary");
        assert_eq!(
            message.message_subtype.as_deref(),
            Some("manual_clear_context")
        );
        assert!(message.message.is_empty());
    }

    #[test]
    fn test_workflow_events_table_and_index_exist_after_migration() {
        let store = create_test_store();
        let conn = store
            .conn
            .lock()
            .expect("failed to lock db connection for schema check");

        let table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM sqlite_master WHERE type = 'table' AND name = 'workflow_events'",
                [],
                |row| row.get(0),
            )
            .expect("failed to query workflow_events table existence");
        assert_eq!(table_exists, 1, "workflow_events table should exist");

        let index_exists: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM sqlite_master WHERE type = 'index' AND name = 'idx_workflow_events_session_id_id'",
                [],
                |row| row.get(0),
            )
            .expect("failed to query workflow_events index existence");
        assert_eq!(
            index_exists, 1,
            "idx_workflow_events_session_id_id index should exist"
        );
    }

    #[test]
    fn test_memory_candidates_table_and_index_exist_after_migration() {
        let store = create_test_store();
        let conn = store
            .conn
            .lock()
            .expect("failed to lock db connection for schema check");

        let table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM sqlite_master WHERE type = 'table' AND name = 'memory_candidates'",
                [],
                |row| row.get(0),
            )
            .expect("failed to query memory_candidates table existence");
        assert_eq!(table_exists, 1, "memory_candidates table should exist");

        let index_exists: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM sqlite_master WHERE type = 'index' AND name = 'idx_memory_candidates_unique_content'",
                [],
                |row| row.get(0),
            )
            .expect("failed to query memory_candidates unique index existence");
        assert_eq!(
            index_exists, 1,
            "memory candidate unique index should exist"
        );
    }

    #[test]
    fn test_append_workflow_event_returns_error_when_table_missing() {
        let store = create_test_store();
        {
            let conn = store
                .conn
                .lock()
                .expect("failed to lock db connection for table drop");
            conn.execute("DROP TABLE workflow_events", [])
                .expect("failed to drop workflow_events table");
        }

        let event = WorkflowEvent::workflow_started(
            "session-append-fail".to_string(),
            "agent-test".to_string(),
        );
        let result = store.append_workflow_event(&event);
        assert!(
            result.is_err(),
            "append_workflow_event should return error when table is missing"
        );
    }

    #[test]
    fn test_snapshot_last_event_id_aligns_with_event_tail_for_key_states() {
        let store = create_test_store();
        let session_id = "session-last-event-align";

        let started =
            WorkflowEvent::workflow_started(session_id.to_string(), "agent-test".to_string());
        let e1 = store
            .append_workflow_event(&started)
            .expect("failed to append workflow_started event");
        let state_changed = WorkflowEvent::state_changed(
            session_id.to_string(),
            "thinking".to_string(),
            "executing".to_string(),
        );
        let e2 = store
            .append_workflow_event(&state_changed)
            .expect("failed to append state_changed event");
        assert!(e2 >= e1, "event ids should be monotonic");

        let expected_last_event_id = store
            .get_last_event_id(session_id)
            .expect("failed to query last event id")
            .expect("last event id should exist after appends");

        let mut waiting_ctx = ExecutionContext::new(session_id.to_string());
        waiting_ctx.state = RuntimeState::Waiting;
        waiting_ctx.wait_reason = Some(WaitReason::Approval);
        waiting_ctx.last_event_id = Some(expected_last_event_id);
        store
            .upsert_execution_context(&waiting_ctx)
            .expect("failed to save waiting snapshot");
        let loaded_waiting = store
            .get_execution_context(session_id)
            .expect("failed to load waiting snapshot")
            .expect("waiting snapshot should exist");
        assert_eq!(
            loaded_waiting.last_event_id,
            Some(expected_last_event_id),
            "waiting snapshot last_event_id should align with event tail"
        );

        let mut completed_ctx = waiting_ctx.clone();
        completed_ctx.state = RuntimeState::Completed;
        completed_ctx.wait_reason = None;
        completed_ctx.last_event_id = Some(expected_last_event_id);
        store
            .upsert_execution_context(&completed_ctx)
            .expect("failed to save completed snapshot");
        let loaded_completed = store
            .get_execution_context(session_id)
            .expect("failed to load completed snapshot")
            .expect("completed snapshot should exist");
        assert_eq!(
            loaded_completed.last_event_id,
            Some(expected_last_event_id),
            "completed snapshot last_event_id should align with event tail"
        );

        let mut cancelled_ctx = completed_ctx.clone();
        cancelled_ctx.state = RuntimeState::Cancelled;
        cancelled_ctx.last_event_id = Some(expected_last_event_id);
        store
            .upsert_execution_context(&cancelled_ctx)
            .expect("failed to save cancelled snapshot");
        let loaded_cancelled = store
            .get_execution_context(session_id)
            .expect("failed to load cancelled snapshot")
            .expect("cancelled snapshot should exist");
        assert_eq!(
            loaded_cancelled.last_event_id,
            Some(expected_last_event_id),
            "cancelled snapshot last_event_id should align with event tail"
        );
    }

    #[test]
    fn test_list_workflows_excludes_child_workflows() {
        let store = create_test_store();
        seed_agent(&store, "agent-parent");
        seed_agent(&store, "agent-child");

        store
            .create_workflow("parent-session", "Parent query", "agent-parent", None, None)
            .expect("failed to create parent workflow");
        store
            .create_workflow(
                "task_legacy_child_session",
                "Legacy child query",
                "agent-child",
                None,
                None,
            )
            .expect("failed to create legacy child workflow");
        store
            .create_workflow(
                "child-session",
                "Child query",
                "agent-child",
                None,
                Some("parent-session"),
            )
            .expect("failed to create child workflow");

        let workflows = store
            .list_workflows()
            .expect("failed to list top-level workflows");

        assert_eq!(workflows.len(), 1);
        assert_eq!(
            workflows[0].id.as_deref(),
            Some("parent-session"),
            "child workflow should not appear in top-level workflow list"
        );
    }

    #[test]
    fn test_list_workflows_marks_automation_runs() {
        let store = create_test_store();
        seed_agent(&store, "agent-main");

        store
            .create_workflow("normal-session", "Normal query", "agent-main", None, None)
            .expect("failed to create normal workflow");
        store
            .create_workflow(
                "automation-session",
                "Automation query",
                "agent-main",
                None,
                None,
            )
            .expect("failed to create automation workflow");

        let conn = store
            .conn
            .lock()
            .expect("failed to lock db connection for automation marker test");
        conn.execute(
            "INSERT INTO workflow_automations
             (id, title, agent_id, allowed_paths, schedule_kind, schedule_config, enabled)
             VALUES (?1, ?2, ?3, '[]', 'daily', '{}', 1)",
            params!["automation-1", "Automation", "agent-main"],
        )
        .expect("failed to insert automation");
        conn.execute(
            "INSERT INTO workflow_automation_runs
             (id, automation_id, workflow_session_id, status, scheduled_for)
             VALUES (?1, ?2, ?3, 'running', '2026-06-27 10:00:00')",
            params!["run-1", "automation-1", "automation-session"],
        )
        .expect("failed to insert automation run");
        drop(conn);

        let workflows = store
            .list_workflows()
            .expect("failed to list top-level workflows");

        let normal = workflows
            .iter()
            .find(|workflow| workflow.id.as_deref() == Some("normal-session"))
            .expect("normal workflow should exist");
        let automation = workflows
            .iter()
            .find(|workflow| workflow.id.as_deref() == Some("automation-session"))
            .expect("automation workflow should exist");

        assert!(!normal.is_automation_run);
        assert!(automation.is_automation_run);
    }

    #[test]
    fn test_get_workflow_snapshot_marks_automation_runs() {
        let store = create_test_store();
        seed_agent(&store, "agent-main");

        store
            .create_workflow(
                "automation-session",
                "Automation query",
                "agent-main",
                None,
                None,
            )
            .expect("failed to create automation workflow");

        let conn = store
            .conn
            .lock()
            .expect("failed to lock db connection for snapshot automation marker test");
        conn.execute(
            "INSERT INTO workflow_automations
             (id, title, agent_id, allowed_paths, schedule_kind, schedule_config, enabled)
             VALUES (?1, ?2, ?3, '[]', 'daily', '{}', 1)",
            params!["automation-1", "Automation", "agent-main"],
        )
        .expect("failed to insert automation");
        conn.execute(
            "INSERT INTO workflow_automation_runs
             (id, automation_id, workflow_session_id, status, scheduled_for)
             VALUES (?1, ?2, ?3, 'running', '2026-06-27 10:00:00')",
            params!["run-1", "automation-1", "automation-session"],
        )
        .expect("failed to insert automation run");
        drop(conn);

        let snapshot = store
            .get_workflow_snapshot("automation-session")
            .expect("failed to get workflow snapshot");

        assert!(snapshot.workflow.is_automation_run);
    }

    #[test]
    fn test_delete_workflow_removes_sub_agent_descendants() {
        let store = create_test_store();
        seed_agent(&store, "agent-parent");
        seed_agent(&store, "agent-child");

        store
            .create_workflow("parent-session", "Parent query", "agent-parent", None, None)
            .expect("failed to create parent workflow");
        store
            .create_workflow(
                "subagent-child",
                "Child query",
                "agent-child",
                None,
                Some("parent-session"),
            )
            .expect("failed to create child workflow");

        let conn = store
            .conn
            .lock()
            .expect("failed to lock db connection for delete assertions");
        for session_id in ["parent-session", "subagent-child"] {
            conn.execute(
                "INSERT INTO workflow_messages (session_id, role, message) VALUES (?1, 'user', 'message')",
                params![session_id],
            )
            .expect("failed to insert workflow message");
            conn.execute(
                "INSERT INTO workflow_context_messages (session_id, segment_id, role, message, source_message_id)
                 VALUES (?1, 1, 'user', 'context', last_insert_rowid())",
                params![session_id],
            )
            .expect("failed to insert workflow context message");
            conn.execute(
                "INSERT INTO workflow_snapshots (session_id, context_json, version)
                 VALUES (?1, '{}', '1')",
                params![session_id],
            )
            .expect("failed to insert workflow snapshot");
            conn.execute(
                "INSERT INTO workflow_events (session_id, event_type, event_version, event_data)
                 VALUES (?1, 'test', '1', '{}')",
                params![session_id],
            )
            .expect("failed to insert workflow event");
        }
        drop(conn);

        store
            .delete_workflow("parent-session")
            .expect("failed to recursively delete workflow tree");

        let conn = store
            .conn
            .lock()
            .expect("failed to lock db connection for delete assertions");
        let workflow_count: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM workflows WHERE id IN ('parent-session', 'subagent-child')",
                [],
                |row| row.get(0),
            )
            .expect("failed to count deleted workflows");
        assert_eq!(workflow_count, 0, "workflow records should be deleted");

        for table in [
            "workflow_messages",
            "workflow_context_messages",
            "workflow_snapshots",
            "workflow_events",
        ] {
            let count: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(1) FROM {table} WHERE session_id IN ('parent-session', 'subagent-child')"),
                    [],
                    |row| row.get(0),
                )
                .expect("failed to count deleted workflow records");
            assert_eq!(count, 0, "{table} records should be deleted");
        }
    }

    #[test]
    fn test_get_workflow_efficiency_report_splits_main_and_sub_agents() {
        let store = create_test_store();
        seed_agent(&store, "agent-main");
        seed_agent(&store, "agent-child");

        store
            .create_workflow("main-session", "Main task", "agent-main", None, None)
            .expect("failed to create main workflow");
        store
            .create_workflow(
                "subagent-child",
                "Explore task",
                "agent-child",
                None,
                Some("main-session"),
            )
            .expect("failed to create sub workflow");

        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: "main-session".to_string(),
                role: "assistant".to_string(),
                message: "Read and edit".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(serde_json::json!({
                    "tool_calls": [
                        {
                            "function": {
                                "name": "read_file",
                                "arguments": "{\"file_path\":\"src/main.rs\"}"
                            }
                        },
                        {
                            "function": {
                                "name": "read_file",
                                "arguments": "{\"file_path\":\"src/lib.rs\"}"
                            }
                        }
                    ]
                })),
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add main assistant context");
        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: "main-session".to_string(),
                role: "tool".to_string(),
                message: "<file_content path=\"src/main.rs\">fn main() {}</file_content>"
                    .to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(serde_json::json!({
                    "tool_name": "read_file",
                    "tool_call": {
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"file_path\":\"src/main.rs\"}"
                        }
                    }
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add main read tool");
        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: "main-session".to_string(),
                role: "tool".to_string(),
                message: "{\"file_path\":\"src/main.rs\"}".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(serde_json::json!({
                    "tool_name": "edit_file",
                    "tool_call": {
                        "function": {
                            "name": "edit_file",
                            "arguments": "{\"file_path\":\"src/main.rs\"}"
                        }
                    }
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 3,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add main edit tool");
        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: "main-session".to_string(),
                role: "tool".to_string(),
                message: "ok".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(serde_json::json!({
                    "tool_name": "bash",
                    "tool_call": {
                        "function": {
                            "name": "bash",
                            "arguments": "{\"command\":\"cargo check\"}"
                        }
                    }
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 4,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add main verification tool");

        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: "subagent-child".to_string(),
                role: "assistant".to_string(),
                message: "Search in parallel".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(serde_json::json!({
                    "tool_calls": [
                        {
                            "function": {
                                "name": "grep",
                                "arguments": "{\"pattern\":\"foo\"}"
                            }
                        },
                        {
                            "function": {
                                "name": "glob",
                                "arguments": "{\"pattern\":\"*.rs\"}"
                            }
                        }
                    ]
                })),
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add sub assistant context");
        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: "subagent-child".to_string(),
                role: "tool".to_string(),
                message: "[No matches found]".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(serde_json::json!({
                    "tool_name": "grep",
                    "tool_call": {
                        "function": {
                            "name": "grep",
                            "arguments": "{\"pattern\":\"foo\"}"
                        }
                    }
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add sub grep tool");

        let report = store
            .get_workflow_efficiency_report("main-session")
            .expect("failed to build efficiency report");

        assert_eq!(report.main_agent.session_id, "main-session");
        assert_eq!(report.sub_agents.len(), 1);
        assert_eq!(report.sub_agents[0].session_id, "subagent-child");
        assert_eq!(report.main_agent.metrics.read_calls, 1);
        assert_eq!(report.main_agent.metrics.edit_calls, 1);
        assert_eq!(report.main_agent.metrics.verification_calls, 1);
        assert_eq!(report.main_agent.metrics.pre_edit_read_coverage, 100);
        assert_eq!(report.sub_agents[0].metrics.parallel_search_rounds, 1);
        assert_eq!(report.sub_agents[0].metrics.no_match_searches, 1);
        assert!(report.main_agent.metrics.convergence_score >= 60);
        assert!(report.main_agent.metrics.execution_score >= 80);
    }

    #[test]
    fn test_delete_last_workflow_message_rewinds_trailing_user_message_and_rebuilds_snapshot() {
        let store = create_test_store();
        let session_id = "session-delete-last-workflow-message";
        seed_agent(&store, "agent-test");

        store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");

        let user_message = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "user".to_string(),
                message: "Fix it".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: None,
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add user message");

        let assistant_message = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "assistant".to_string(),
                message: "I'll edit the file".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(serde_json::json!({
                    "tool_calls": [{
                        "id": "tool_1",
                        "function": {
                            "name": "edit_file",
                            "arguments": "{\"file_path\":\"a.rs\"}"
                        }
                    }]
                })),
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add assistant message");

        let tool_message = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "tool".to_string(),
                message: "{\"file_path\":\"a.rs\"}".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(serde_json::json!({
                    "tool_call_id": "tool_1",
                    "tool_name": "edit_file",
                    "approval_status": "pending",
                    "execution_status": "pending_approval"
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 3,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add tool message");

        store
            .add_workflow_ai_context_message(&WorkflowAiContextMessage {
                id: None,
                session_id: session_id.to_string(),
                segment_id: 1,
                role: "assistant".to_string(),
                message: assistant_message.message.clone(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                metadata: assistant_message.metadata.clone(),
                source_message_id: assistant_message.id,
                created_at: None,
            })
            .expect("failed to add assistant context message");
        store
            .add_workflow_ai_context_message(&WorkflowAiContextMessage {
                id: None,
                session_id: session_id.to_string(),
                segment_id: 1,
                role: "tool".to_string(),
                message: tool_message.message.clone(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                metadata: tool_message.metadata.clone(),
                source_message_id: tool_message.id,
                created_at: None,
            })
            .expect("failed to add tool context message");

        let started =
            WorkflowEvent::workflow_started(session_id.to_string(), "agent-test".to_string());
        store
            .append_workflow_event(&started)
            .expect("failed to append workflow_started event");
        let wait_entered = WorkflowEvent::wait_entered(
            session_id.to_string(),
            "approval".to_string(),
            vec![json!({
                "tool_call_id": "tool_1",
                "tool_name": "edit_file",
                "arguments": { "file_path": "a.rs" },
                "details": Value::Null,
                "display_type": "text"
            })],
        );
        store
            .append_workflow_event(&wait_entered)
            .expect("failed to append wait_entered event");
        let approval_requested = WorkflowEvent::approval_requested(
            session_id.to_string(),
            "tool_1".to_string(),
            "edit_file".to_string(),
            json!({ "file_path": "a.rs" }),
            None,
            Some("text".to_string()),
        );
        let last_event_id = store
            .append_workflow_event(&approval_requested)
            .expect("failed to append approval_requested event");

        let mut context = ExecutionContext::new(session_id.to_string());
        context.state = RuntimeState::Waiting;
        context.wait_reason = Some(WaitReason::Approval);
        context.pending_tools = vec![PendingTool {
            tool_call_id: "tool_1".to_string(),
            tool_name: "edit_file".to_string(),
            arguments: json!({ "file_path": "a.rs" }),
            details: None,
            display_type: Some("text".to_string()),
        }];
        context.last_event_id = Some(last_event_id);
        store
            .upsert_execution_context(&context)
            .expect("failed to persist snapshot");

        let trailing_user_message = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "user".to_string(),
                message: "Continue from here".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 2,
                source_event_type: None,
                metadata: None,
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 4,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add trailing user message");

        let deleted = store
            .delete_last_message(session_id)
            .expect("failed to delete last workflow message");
        assert!(deleted, "tail message should be deleted");

        let snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to load workflow snapshot after deletion");
        assert_eq!(snapshot.messages.len(), 3);
        assert_eq!(snapshot.messages[0].id, user_message.id);
        assert_eq!(snapshot.messages[0].role, "user");
        assert_eq!(snapshot.messages[1].id, assistant_message.id);
        assert_eq!(snapshot.messages[1].role, "assistant");
        assert_eq!(snapshot.messages[2].id, tool_message.id);
        assert_eq!(snapshot.messages[2].role, "tool");
        assert!(snapshot
            .messages
            .iter()
            .all(|message| message.id != trailing_user_message.id));

        let conn = store
            .conn
            .lock()
            .expect("failed to lock db connection for assertions");

        let context_count: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM workflow_context_messages WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .expect("failed to count context messages");
        assert_eq!(context_count, 0);

        let event_count: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM workflow_events WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .expect("failed to count events");
        assert_eq!(event_count, 3);

        let status: String = conn
            .query_row(
                "SELECT status FROM workflows WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .expect("failed to read workflow status");
        assert_eq!(status, "awaiting_approval");
        drop(conn);

        let restored = store
            .get_execution_context(session_id)
            .expect("failed to load rebuilt snapshot")
            .expect("rebuilt snapshot should exist");
        assert_eq!(restored.wait_reason, Some(WaitReason::Approval));
        assert_eq!(restored.pending_tools.len(), 1);
        assert_eq!(restored.last_event_id, Some(last_event_id));
    }

    #[test]
    fn test_delete_last_workflow_message_rewinds_continuation_state_to_prior_completion() {
        let store = create_test_store();
        let session_id = "session-delete-last-continuation-after-completion";
        seed_agent(&store, "agent-test");

        store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");

        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "tool".to_string(),
                message: "Task completed".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: Some("tool_completed".to_string()),
                metadata: Some(json!({
                    "tool_call_id": "complete-1",
                    "tool_name": crate::tools::TOOL_COMPLETE_WORKFLOW,
                    "approval_status": "approved",
                    "execution_status": "completed"
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add completion message");

        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "user".to_string(),
                message: "Continue the conversation".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 2,
                source_event_type: None,
                metadata: None,
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add continuation message");

        store
            .append_workflow_event(&WorkflowEvent::workflow_started(
                session_id.to_string(),
                "agent-test".to_string(),
            ))
            .expect("failed to append workflow_started event");
        let completed_event_id = store
            .append_workflow_event(&WorkflowEvent::workflow_completed(
                session_id.to_string(),
                Some("Task completed".to_string()),
            ))
            .expect("failed to append workflow_completed event");
        store
            .append_workflow_event(&WorkflowEvent::user_input_received(
                session_id.to_string(),
                "Continue the conversation".to_string(),
            ))
            .expect("failed to append user_input_received event");
        store
            .append_workflow_event(&WorkflowEvent::state_changed(
                session_id.to_string(),
                "completed".to_string(),
                "thinking".to_string(),
            ))
            .expect("failed to append continuation state event");

        let deleted = store
            .delete_last_message(session_id)
            .expect("failed to delete continuation message");
        assert!(deleted);

        let snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to load rewound workflow snapshot");
        assert_eq!(snapshot.messages.len(), 1);
        assert_eq!(snapshot.workflow.status, "completed");

        let restored = store
            .get_execution_context(session_id)
            .expect("failed to load rewound execution context")
            .expect("rewound execution context should exist");
        assert_eq!(restored.state, RuntimeState::Completed);
        assert_eq!(restored.last_event_id, Some(completed_event_id));
    }

    #[test]
    fn test_delete_last_workflow_message_rewinds_answered_ask_user_in_two_steps() {
        let store = create_test_store();
        let session_id = "session-delete-last-answered-ask-user";
        seed_agent(&store, "agent-test");

        store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");

        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "user".to_string(),
                message: "Help me decide".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: None,
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add user message");
        let assistant_ask_user_placeholder = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "assistant".to_string(),
                message: "I need one more choice from you before continuing.".to_string(),
                reasoning: Some("Ask the user to choose an option.".to_string()),
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_calls": [{
                        "id": "ask_user_1",
                        "type": "function",
                        "function": {
                            "name": crate::tools::TOOL_ASK_USER,
                            "arguments": "{\"items\":[{\"title\":\"Choose\",\"options\":[\"A\",\"B\"]}]}"
                        }
                    }]
                })),
                attached_context: None,
                step_type: Some("act".to_string()),
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add assistant ask_user placeholder");
        let ask_user_message = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "tool".to_string(),
                message: "[{\"title\":\"Choose\",\"options\":[\"A\",\"B\"]}]".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_call_id": "ask_user_1",
                    "tool_name": crate::tools::TOOL_ASK_USER,
                    "display_type": "choice",
                    "execution_status": "completed"
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 3,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add ask_user message");
        let user_answer = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "user".to_string(),
                message: "I choose A".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: None,
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 4,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add user answer");

        store
            .append_workflow_event(&WorkflowEvent::workflow_started(
                session_id.to_string(),
                "agent-test".to_string(),
            ))
            .expect("failed to append workflow_started");
        let ask_user_wait_id = store
            .append_workflow_event(&WorkflowEvent::wait_entered(
                session_id.to_string(),
                "user_input".to_string(),
                Vec::new(),
            ))
            .expect("failed to append ask_user wait");
        let user_input_received_id = store
            .append_workflow_event(&WorkflowEvent::user_input_received(
                session_id.to_string(),
                "I choose A".to_string(),
            ))
            .expect("failed to append user_input_received");

        let mut context = ExecutionContext::new(session_id.to_string());
        context.state = RuntimeState::Running;
        context.last_event_id = Some(user_input_received_id);
        store
            .upsert_execution_context(&context)
            .expect("failed to persist running snapshot");

        assert!(store
            .delete_last_message(session_id)
            .expect("failed to rewind answered ask_user"));

        let first_pass_snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to reload after first rewind");
        assert_eq!(first_pass_snapshot.messages.len(), 3);
        assert!(first_pass_snapshot
            .messages
            .iter()
            .any(|message| message.id == ask_user_message.id));
        assert!(first_pass_snapshot
            .messages
            .iter()
            .any(|message| message.id == assistant_ask_user_placeholder.id));
        assert!(first_pass_snapshot
            .messages
            .iter()
            .all(|message| message.id != user_answer.id));

        let first_context = store
            .get_execution_context(session_id)
            .expect("failed to load first rewind snapshot")
            .expect("snapshot should exist after first rewind");
        assert_eq!(first_context.state, RuntimeState::Waiting);
        assert_eq!(first_context.wait_reason, Some(WaitReason::UserInput));
        assert_eq!(first_context.last_event_id, Some(ask_user_wait_id));

        assert!(store
            .delete_last_message(session_id)
            .expect("failed to rewind ask_user wait"));

        let second_pass_snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to reload after second rewind");
        assert_eq!(second_pass_snapshot.messages.len(), 1);
        assert!(second_pass_snapshot
            .messages
            .iter()
            .all(|message| message.id != ask_user_message.id));
        assert!(second_pass_snapshot
            .messages
            .iter()
            .all(|message| message.id != assistant_ask_user_placeholder.id));

        let second_context = store
            .get_execution_context(session_id)
            .expect("failed to load second rewind snapshot")
            .expect("snapshot should exist after second rewind");
        assert_eq!(second_context.state, RuntimeState::Running);
        assert_eq!(second_context.wait_reason, None);
    }

    #[test]
    fn test_delete_last_workflow_message_rewinds_unanswered_ask_user_batch_after_stop() {
        let store = create_test_store();
        let session_id = "session-delete-last-unanswered-ask-user-stop";
        seed_agent(&store, "agent-test");

        store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");

        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "user".to_string(),
                message: "Need a decision".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: None,
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add user message");
        let assistant_ask_user_placeholder = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "assistant".to_string(),
                message: "I need one more answer from you before proceeding.".to_string(),
                reasoning: Some("Pause and ask the user to choose a direction.".to_string()),
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_calls": [{
                        "id": "ask_user_stop_1",
                        "type": "function",
                        "function": {
                            "name": crate::tools::TOOL_ASK_USER,
                            "arguments": "{\"items\":[{\"title\":\"Choose\",\"options\":[\"A\",\"B\"]}]}"
                        }
                    }]
                })),
                attached_context: None,
                step_type: Some("act".to_string()),
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add assistant ask_user placeholder");
        let ask_user_message = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "tool".to_string(),
                message: "[{\"title\":\"Choose\",\"options\":[\"A\",\"B\"]}]".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_call_id": "ask_user_stop_1",
                    "tool_name": crate::tools::TOOL_ASK_USER,
                    "display_type": "choice",
                    "execution_status": "completed"
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 3,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add ask_user message");
        store
            .append_workflow_event(&WorkflowEvent::workflow_started(
                session_id.to_string(),
                "agent-test".to_string(),
            ))
            .expect("failed to append workflow_started");
        let wait_event_id = store
            .append_workflow_event(&WorkflowEvent::wait_entered(
                session_id.to_string(),
                "user_input".to_string(),
                Vec::new(),
            ))
            .expect("failed to append ask_user wait");

        let mut context = ExecutionContext::new(session_id.to_string());
        context.state = RuntimeState::Cancelled;
        context.wait_reason = Some(WaitReason::UserInput);
        context.last_event_id = Some(wait_event_id);
        store
            .upsert_execution_context(&context)
            .expect("failed to persist cancelled snapshot");

        assert!(store
            .delete_last_message(session_id)
            .expect("failed to rewind unanswered ask_user after stop"));

        let snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to reload after rewind");
        assert_eq!(snapshot.messages.len(), 1);
        assert!(snapshot
            .messages
            .iter()
            .all(|message| message.id != ask_user_message.id));
        assert!(snapshot
            .messages
            .iter()
            .all(|message| message.id != assistant_ask_user_placeholder.id));

        let restored = store
            .get_execution_context(session_id)
            .expect("failed to load rebuilt snapshot")
            .expect("snapshot should exist after rewind");
        assert_eq!(restored.state, RuntimeState::Running);
        assert_eq!(restored.wait_reason, None);
    }

    #[test]
    fn test_delete_last_workflow_message_rewinds_pending_submit_plan_to_planning() {
        let store = create_test_store();
        let session_id = "session-delete-last-pending-submit-plan";
        seed_agent(&store, "agent-test");

        store
            .create_workflow(
                session_id,
                "Initial query",
                "agent-test",
                Some(json!({ "phase": "planning" }).to_string()),
                None,
            )
            .expect("failed to create workflow");

        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "user".to_string(),
                message: "Plan this change".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: None,
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add user message");

        let pending_plan_message = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "tool".to_string(),
                message: "Approved execution plan draft".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_call_id": "submit_plan_1",
                    "tool_name": crate::tools::TOOL_SUBMIT_PLAN,
                    "approval_status": "pending",
                    "execution_status": "pending_approval",
                    "display_type": "markdown"
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add pending submit_plan message");
        assert!(pending_plan_message.id.is_some());

        let started =
            WorkflowEvent::workflow_started(session_id.to_string(), "agent-test".to_string());
        store
            .append_workflow_event(&started)
            .expect("failed to append workflow_started event");
        store
            .append_workflow_event(&WorkflowEvent::wait_entered(
                session_id.to_string(),
                "approval".to_string(),
                vec![json!({
                    "tool_call_id": "submit_plan_1",
                    "tool_name": crate::tools::TOOL_SUBMIT_PLAN,
                    "arguments": { "plan": "# Plan" },
                    "details": Value::Null,
                    "display_type": "markdown"
                })],
            ))
            .expect("failed to append approval wait event");
        store
            .append_workflow_event(&WorkflowEvent::approval_requested(
                session_id.to_string(),
                "submit_plan_1".to_string(),
                crate::tools::TOOL_SUBMIT_PLAN.to_string(),
                json!({ "plan": "# Plan" }),
                None,
                Some("markdown".to_string()),
            ))
            .expect("failed to append approval_requested event");

        let deleted = store
            .delete_last_message(session_id)
            .expect("failed to rewind pending submit_plan");
        assert!(deleted);

        let snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to reload workflow after rewind");
        assert_eq!(snapshot.messages.len(), 1);

        let restored = store
            .get_execution_context(session_id)
            .expect("failed to load snapshot after rewind")
            .expect("snapshot should exist after rewind");
        assert_eq!(restored.state, RuntimeState::Running);
        assert_eq!(restored.wait_reason, None);

        let conn = store
            .conn
            .lock()
            .expect("failed to lock db connection for assertions");
        let status: String = conn
            .query_row(
                "SELECT status FROM workflows WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .expect("failed to load workflow status");
        assert_eq!(status, "thinking");
        let agent_config: String = conn
            .query_row(
                "SELECT agent_config FROM workflows WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .expect("failed to load agent_config");
        assert_eq!(
            serde_json::from_str::<Value>(&agent_config)
                .expect("agent config should be valid json")["phase"]
                .as_str(),
            Some("planning")
        );
    }

    #[test]
    fn test_delete_last_workflow_message_rewinds_pending_approval_tool_to_running() {
        let store = create_test_store();
        let session_id = "session-delete-last-pending-approval-tool";
        seed_agent(&store, "agent-test");

        store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");

        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "user".to_string(),
                message: "Edit config".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: None,
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add user message");
        let assistant_placeholder = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "assistant".to_string(),
                message: "I will edit the config now.".to_string(),
                reasoning: Some("Preparing the single edit tool call.".to_string()),
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_calls": [{
                        "id": "edit_1",
                        "type": "function",
                        "function": {
                            "name": "edit_file",
                            "arguments": "{\"file_path\":\"app.toml\"}"
                        }
                    }]
                })),
                attached_context: None,
                step_type: Some("act".to_string()),
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add assistant placeholder");
        let pending_tool = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "tool".to_string(),
                message: "{\"file_path\":\"app.toml\"}".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_call_id": "edit_1",
                    "tool_name": "edit_file",
                    "approval_status": "pending",
                    "execution_status": "pending_approval",
                    "display_type": "text"
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 3,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add pending approval tool");

        store
            .append_workflow_event(&WorkflowEvent::workflow_started(
                session_id.to_string(),
                "agent-test".to_string(),
            ))
            .expect("failed to append workflow_started");
        store
            .append_workflow_event(&WorkflowEvent::wait_entered(
                session_id.to_string(),
                "approval".to_string(),
                vec![json!({
                    "tool_call_id": "edit_1",
                    "tool_name": "edit_file",
                    "arguments": { "file_path": "app.toml" },
                    "details": Value::Null,
                    "display_type": "text"
                })],
            ))
            .expect("failed to append approval wait");
        store
            .append_workflow_event(&WorkflowEvent::approval_requested(
                session_id.to_string(),
                "edit_1".to_string(),
                "edit_file".to_string(),
                json!({ "file_path": "app.toml" }),
                None,
                Some("text".to_string()),
            ))
            .expect("failed to append approval requested");

        let mut context = ExecutionContext::new(session_id.to_string());
        context.state = RuntimeState::Waiting;
        context.wait_reason = Some(WaitReason::Approval);
        context.pending_tools = vec![PendingTool {
            tool_call_id: "edit_1".to_string(),
            tool_name: "edit_file".to_string(),
            arguments: json!({ "file_path": "app.toml" }),
            details: None,
            display_type: Some("text".to_string()),
        }];
        store
            .upsert_execution_context(&context)
            .expect("failed to persist approval wait snapshot");

        assert!(store
            .delete_last_message(session_id)
            .expect("failed to rewind pending approval tool"));

        let snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to load snapshot after rewind");
        assert_eq!(snapshot.messages.len(), 1);
        assert!(snapshot
            .messages
            .iter()
            .all(|message| message.id != pending_tool.id));
        assert!(snapshot
            .messages
            .iter()
            .all(|message| message.id != assistant_placeholder.id));

        let rebuilt = store
            .get_execution_context(session_id)
            .expect("failed to load rebuilt snapshot")
            .expect("rebuilt snapshot should exist");
        assert_eq!(rebuilt.state, RuntimeState::Running);
        assert_eq!(rebuilt.wait_reason, None);
    }

    #[test]
    fn test_delete_last_workflow_message_rewinds_completed_tool_to_running() {
        let store = create_test_store();
        let session_id = "session-delete-last-completed-tool";
        seed_agent(&store, "agent-test");

        store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");

        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "user".to_string(),
                message: "Read the file".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: None,
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add user message");
        let assistant_placeholder = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "assistant".to_string(),
                message: "I will read README.md.".to_string(),
                reasoning: Some("Preparing the read tool call.".to_string()),
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_calls": [{
                        "id": "read_1",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"file_path\":\"README.md\"}"
                        }
                    }]
                })),
                attached_context: None,
                step_type: Some("act".to_string()),
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add assistant placeholder");
        let completed_tool = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "tool".to_string(),
                message: "file contents".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_call_id": "read_1",
                    "tool_name": "read_file",
                    "execution_status": "completed",
                    "display_type": "text"
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 3,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add completed tool message");

        store
            .append_workflow_event(&WorkflowEvent::workflow_started(
                session_id.to_string(),
                "agent-test".to_string(),
            ))
            .expect("failed to append workflow_started");
        store
            .append_workflow_event(&WorkflowEvent::tool_started(
                session_id.to_string(),
                "read_1".to_string(),
                "read_file".to_string(),
                json!({ "file_path": "README.md" }),
            ))
            .expect("failed to append tool_started");
        store
            .append_workflow_event(&WorkflowEvent::tool_completed(
                session_id.to_string(),
                "read_1".to_string(),
                "read_file".to_string(),
                Some(json!("file contents")),
            ))
            .expect("failed to append tool_completed");

        let mut context = ExecutionContext::new(session_id.to_string());
        context.state = RuntimeState::Running;
        store
            .upsert_execution_context(&context)
            .expect("failed to persist running snapshot");

        assert!(store
            .delete_last_message(session_id)
            .expect("failed to rewind completed tool"));

        let snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to load snapshot after rewind");
        assert_eq!(snapshot.messages.len(), 1);
        assert!(snapshot
            .messages
            .iter()
            .all(|message| message.id != completed_tool.id));
        assert!(snapshot
            .messages
            .iter()
            .all(|message| message.id != assistant_placeholder.id));

        let rebuilt = store
            .get_execution_context(session_id)
            .expect("failed to load rebuilt snapshot")
            .expect("rebuilt snapshot should exist");
        assert_eq!(rebuilt.state, RuntimeState::Running);
        assert_eq!(rebuilt.wait_reason, None);
    }

    #[test]
    fn test_delete_last_workflow_message_deletes_completed_approved_tool_unit() {
        let store = create_test_store();
        let session_id = "session-delete-last-approved-tool";
        seed_agent(&store, "agent-test");

        store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");

        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "user".to_string(),
                message: "Edit config".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: None,
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add user message");
        let assistant_placeholder = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "assistant".to_string(),
                message: "I will edit the config now.".to_string(),
                reasoning: Some("Preparing the single edit tool call.".to_string()),
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_calls": [{
                        "id": "edit_2",
                        "type": "function",
                        "function": {
                            "name": "edit_file",
                            "arguments": "{\"file_path\":\"app.toml\"}"
                        }
                    }]
                })),
                attached_context: None,
                step_type: Some("act".to_string()),
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add assistant placeholder");
        let approval_submitted_tool = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "tool".to_string(),
                message: "{\"file_path\":\"app.toml\"}".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_call_id": "edit_2",
                    "tool_name": "edit_file",
                    "approval_status": "approved",
                    "execution_status": "approval_submitted",
                    "summary": "Executing"
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 3,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add approval_submitted message");
        let completed_tool = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "tool".to_string(),
                message: "updated file".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_call_id": "edit_2",
                    "tool_name": "edit_file",
                    "approval_status": "approved",
                    "execution_status": "completed",
                    "display_type": "text"
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 4,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add completed tool message");

        store
            .append_workflow_event(&WorkflowEvent::workflow_started(
                session_id.to_string(),
                "agent-test".to_string(),
            ))
            .expect("failed to append workflow_started");
        store
            .append_workflow_event(&WorkflowEvent::wait_entered(
                session_id.to_string(),
                "approval".to_string(),
                vec![json!({
                    "tool_call_id": "edit_2",
                    "tool_name": "edit_file",
                    "arguments": { "file_path": "app.toml" },
                    "details": Value::Null,
                    "display_type": "text"
                })],
            ))
            .expect("failed to append approval wait");
        store
            .append_workflow_event(&WorkflowEvent::approval_requested(
                session_id.to_string(),
                "edit_2".to_string(),
                "edit_file".to_string(),
                json!({ "file_path": "app.toml" }),
                None,
                Some("text".to_string()),
            ))
            .expect("failed to append approval requested");
        store
            .append_workflow_event(&WorkflowEvent::approval_resolved(
                session_id.to_string(),
                "edit_2".to_string(),
                crate::tools::TOOL_EDIT_FILE.to_string(),
                true,
                false,
                Some("approved".to_string()),
                Some("approval_submitted".to_string()),
                None,
            ))
            .expect("failed to append approval resolved");
        let tool_started_id = store
            .append_workflow_event(&WorkflowEvent::tool_started(
                session_id.to_string(),
                "edit_2".to_string(),
                "edit_file".to_string(),
                json!({ "file_path": "app.toml" }),
            ))
            .expect("failed to append tool_started");
        store
            .append_workflow_event(&WorkflowEvent::tool_completed(
                session_id.to_string(),
                "edit_2".to_string(),
                "edit_file".to_string(),
                Some(json!("updated file")),
            ))
            .expect("failed to append tool_completed");

        let mut context = ExecutionContext::new(session_id.to_string());
        context.state = RuntimeState::Running;
        context.last_event_id = Some(tool_started_id);
        store
            .upsert_execution_context(&context)
            .expect("failed to persist running snapshot");

        assert!(store
            .delete_last_message(session_id)
            .expect("failed to rewind completed approved tool"));

        let first_snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to load snapshot after first rewind");
        assert_eq!(first_snapshot.messages.len(), 1);
        assert!(first_snapshot
            .messages
            .iter()
            .all(|message| message.id != approval_submitted_tool.id));
        assert!(first_snapshot
            .messages
            .iter()
            .all(|message| message.id != completed_tool.id));
        assert!(first_snapshot
            .messages
            .iter()
            .all(|message| message.id != assistant_placeholder.id));

        let rebuilt = store
            .get_execution_context(session_id)
            .expect("failed to load rebuilt snapshot")
            .expect("rebuilt snapshot should exist");
        assert_eq!(rebuilt.state, RuntimeState::Running);
        assert_eq!(rebuilt.wait_reason, None);
        assert!(rebuilt.pending_tools.is_empty());
    }

    #[test]
    fn test_delete_last_workflow_message_deletes_approved_tool_waiting_execution_unit() {
        let store = create_test_store();
        let session_id = "session-delete-last-approved-tool-waiting-execution";
        seed_agent(&store, "agent-test");

        store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");

        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "user".to_string(),
                message: "Edit config".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: None,
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add user message");
        let assistant_placeholder = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "assistant".to_string(),
                message: "I will edit the config now.".to_string(),
                reasoning: Some("Preparing the single edit tool call.".to_string()),
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_calls": [{
                        "id": "edit_wait_1",
                        "type": "function",
                        "function": {
                            "name": "edit_file",
                            "arguments": "{\"file_path\":\"app.toml\"}"
                        }
                    }]
                })),
                attached_context: None,
                step_type: Some("act".to_string()),
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add assistant placeholder");
        let approval_submitted_tool = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "tool".to_string(),
                message: "{\"file_path\":\"app.toml\"}".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_call_id": "edit_wait_1",
                    "tool_name": "edit_file",
                    "approval_status": "approved",
                    "execution_status": "approval_submitted",
                    "summary": "Executing"
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 3,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add approval_submitted message");

        store
            .append_workflow_event(&WorkflowEvent::workflow_started(
                session_id.to_string(),
                "agent-test".to_string(),
            ))
            .expect("failed to append workflow_started");
        store
            .append_workflow_event(&WorkflowEvent::wait_entered(
                session_id.to_string(),
                "approval".to_string(),
                vec![json!({
                    "tool_call_id": "edit_wait_1",
                    "tool_name": "edit_file",
                    "arguments": { "file_path": "app.toml" },
                    "details": Value::Null,
                    "display_type": "text"
                })],
            ))
            .expect("failed to append approval wait");
        store
            .append_workflow_event(&WorkflowEvent::approval_requested(
                session_id.to_string(),
                "edit_wait_1".to_string(),
                "edit_file".to_string(),
                json!({ "file_path": "app.toml" }),
                None,
                Some("text".to_string()),
            ))
            .expect("failed to append approval requested");
        store
            .append_workflow_event(&WorkflowEvent::approval_resolved(
                session_id.to_string(),
                "edit_wait_1".to_string(),
                crate::tools::TOOL_EDIT_FILE.to_string(),
                true,
                false,
                Some("approved".to_string()),
                Some("approval_submitted".to_string()),
                None,
            ))
            .expect("failed to append approval resolved");

        let mut context = ExecutionContext::new(session_id.to_string());
        context.state = RuntimeState::Running;
        store
            .upsert_execution_context(&context)
            .expect("failed to persist running snapshot");

        assert!(store
            .delete_last_message(session_id)
            .expect("failed to delete approved tool waiting execution"));

        let snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to load snapshot after delete");
        assert_eq!(snapshot.messages.len(), 1);
        assert!(snapshot
            .messages
            .iter()
            .all(|message| message.id != approval_submitted_tool.id));
        assert!(snapshot
            .messages
            .iter()
            .all(|message| message.id != assistant_placeholder.id));

        let rebuilt = store
            .get_execution_context(session_id)
            .expect("failed to load rebuilt snapshot")
            .expect("rebuilt snapshot should exist");
        assert_eq!(rebuilt.state, RuntimeState::Running);
        assert_eq!(rebuilt.wait_reason, None);
        assert!(rebuilt.pending_tools.is_empty());
    }

    #[test]
    fn test_delete_last_workflow_message_rewinds_tail_in_two_steps_after_plan_approval() {
        let store = create_test_store();
        let session_id = "session-delete-last-approved-plan-tail";
        seed_agent(&store, "agent-test");

        store
            .create_workflow(
                session_id,
                "Initial query",
                "agent-test",
                Some(json!({ "phase": "implementation" }).to_string()),
                None,
            )
            .expect("failed to create workflow");

        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "user".to_string(),
                message: "Make the docs changes".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: None,
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add user message");
        let pending_submit_plan = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "tool".to_string(),
                message: "Pending plan".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_call_id": "submit_plan_1",
                    "tool_name": crate::tools::TOOL_SUBMIT_PLAN,
                    "approval_status": "pending",
                    "execution_status": "pending_approval",
                    "display_type": "markdown"
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add pending submit_plan");
        let approved_plan_summary = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "system".to_string(),
                message: "APPROVED EXECUTION PLAN".to_string(),
                reasoning: None,
                message_kind: "summary".to_string(),
                message_subtype: Some("approved_plan".to_string()),
                segment_id: 2,
                source_event_type: None,
                metadata: Some(json!({
                    "type": "summary",
                    "subtype": "approved_plan"
                })),
                attached_context: None,
                step_type: None,
                step_index: 3,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add approved plan summary");
        let approved_submit_plan = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "tool".to_string(),
                message: "Approved plan".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 2,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_call_id": "submit_plan_1",
                    "tool_name": crate::tools::TOOL_SUBMIT_PLAN,
                    "approval_status": "approved",
                    "execution_status": "completed",
                    "display_type": "markdown"
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 3,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add approved submit_plan");
        let assistant_ask_user_placeholder = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "assistant".to_string(),
                message: "".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 2,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_calls": [{
                        "id": "ask_user_1",
                        "type": "function",
                        "function": {
                            "name": crate::tools::TOOL_ASK_USER,
                            "arguments": "{\"items\":[{\"title\":\"Choose\",\"options\":[\"A\",\"B\"]}]}"
                        }
                    }]
                })),
                attached_context: None,
                step_type: Some("act".to_string()),
                step_index: 4,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add assistant ask_user placeholder");
        let ask_user_message = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "tool".to_string(),
                message: "[{\"title\":\"Choose\",\"options\":[\"A\",\"B\"]}]".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 2,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_call_id": "ask_user_1",
                    "tool_name": crate::tools::TOOL_ASK_USER,
                    "display_type": "choice",
                    "execution_status": "completed"
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 4,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add ask_user message");

        store
            .append_workflow_event(&WorkflowEvent::workflow_started(
                session_id.to_string(),
                "agent-test".to_string(),
            ))
            .expect("failed to append workflow_started");
        store
            .append_workflow_event(&WorkflowEvent::wait_entered(
                session_id.to_string(),
                "approval".to_string(),
                vec![json!({
                    "tool_call_id": "submit_plan_1",
                    "tool_name": crate::tools::TOOL_SUBMIT_PLAN,
                    "arguments": { "plan": "# Plan" },
                    "details": Value::Null,
                    "display_type": "markdown"
                })],
            ))
            .expect("failed to append approval wait");
        store
            .append_workflow_event(&WorkflowEvent::approval_requested(
                session_id.to_string(),
                "submit_plan_1".to_string(),
                crate::tools::TOOL_SUBMIT_PLAN.to_string(),
                json!({ "plan": "# Plan" }),
                None,
                Some("markdown".to_string()),
            ))
            .expect("failed to append approval requested");
        store
            .append_workflow_event(&WorkflowEvent::approval_resolved(
                session_id.to_string(),
                "submit_plan_1".to_string(),
                crate::tools::TOOL_SUBMIT_PLAN.to_string(),
                true,
                false,
                Some("approved".to_string()),
                Some("completed".to_string()),
                None,
            ))
            .expect("failed to append approval resolved");
        let tool_started_id = store
            .append_workflow_event(&WorkflowEvent::tool_started(
                session_id.to_string(),
                "submit_plan_1".to_string(),
                crate::tools::TOOL_SUBMIT_PLAN.to_string(),
                json!({ "plan": "# Plan" }),
            ))
            .expect("failed to append tool_started");
        let ask_user_wait_id = store
            .append_workflow_event(&WorkflowEvent::wait_entered(
                session_id.to_string(),
                "user_input".to_string(),
                Vec::new(),
            ))
            .expect("failed to append ask_user wait");

        let mut context = ExecutionContext::new(session_id.to_string());
        context.state = RuntimeState::Waiting;
        context.wait_reason = Some(WaitReason::UserInput);
        context.last_event_id = Some(ask_user_wait_id);
        store
            .upsert_execution_context(&context)
            .expect("failed to persist user wait snapshot");

        assert!(store
            .delete_last_message(session_id)
            .expect("failed to rewind ask_user tail"));

        let first_pass_snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to reload after first rewind");
        assert_eq!(first_pass_snapshot.messages.len(), 4);
        assert!(first_pass_snapshot
            .messages
            .iter()
            .all(|message| message.id != ask_user_message.id));
        assert!(first_pass_snapshot
            .messages
            .iter()
            .all(|message| message.id != assistant_ask_user_placeholder.id));
        assert!(first_pass_snapshot
            .messages
            .iter()
            .any(|message| message.id == approved_submit_plan.id));
        assert!(first_pass_snapshot
            .messages
            .iter()
            .any(|message| message.id == approved_plan_summary.id));

        let first_context = store
            .get_execution_context(session_id)
            .expect("failed to load first rewind snapshot")
            .expect("snapshot should exist after first rewind");
        assert_eq!(first_context.state, RuntimeState::Running);
        assert_eq!(first_context.wait_reason, None);
        assert_eq!(first_context.last_event_id, Some(tool_started_id));

        assert!(store
            .delete_last_message(session_id)
            .expect("failed to rewind approved plan"));

        let second_pass_snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to reload after second rewind");
        assert_eq!(second_pass_snapshot.messages.len(), 2);
        assert!(second_pass_snapshot
            .messages
            .iter()
            .all(|message| message.id != approved_submit_plan.id));
        assert!(second_pass_snapshot
            .messages
            .iter()
            .all(|message| message.id != approved_plan_summary.id));
        assert!(second_pass_snapshot
            .messages
            .iter()
            .any(|message| message.id == pending_submit_plan.id));

        let second_context = store
            .get_execution_context(session_id)
            .expect("failed to load second rewind snapshot")
            .expect("snapshot should exist after second rewind");
        assert_eq!(second_context.wait_reason, Some(WaitReason::Approval));
        assert_eq!(second_context.pending_tools.len(), 1);

        let conn = store
            .conn
            .lock()
            .expect("failed to lock db connection for final assertions");
        let status: String = conn
            .query_row(
                "SELECT status FROM workflows WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .expect("failed to load final workflow status");
        assert_eq!(status, "awaiting_approval");
        let agent_config: String = conn
            .query_row(
                "SELECT agent_config FROM workflows WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .expect("failed to load final agent config");
        assert_eq!(
            serde_json::from_str::<Value>(&agent_config)
                .expect("agent config should be valid json")["phase"]
                .as_str(),
            Some("planning")
        );
    }

    #[test]
    fn test_delete_last_workflow_message_deletes_manual_clear_context_marker_only() {
        let store = create_test_store();
        let session_id = "session-delete-last-manual-clear-context";
        seed_agent(&store, "agent-test");

        store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");

        let user_message = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "user".to_string(),
                message: "Original task".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: None,
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add user message");
        let clear_marker = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "system".to_string(),
                message: "".to_string(),
                reasoning: None,
                message_kind: "summary".to_string(),
                message_subtype: Some("manual_clear_context".to_string()),
                segment_id: 2,
                source_event_type: None,
                metadata: Some(json!({
                    "type": "summary",
                    "subtype": "manual_clear_context",
                    "compressed_until_message_id": user_message.id,
                    "previous_segment_id": 1,
                    "previous_context_tokens": 17753,
                    "previous_max_context_tokens": 202752
                })),
                attached_context: None,
                step_type: None,
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add clear-context marker");

        let _preserved_context = store
            .add_workflow_ai_context_message(&WorkflowAiContextMessage {
                id: None,
                session_id: session_id.to_string(),
                segment_id: 1,
                role: "user".to_string(),
                message: "<user_query>\nOriginal task\n</user_query>".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                metadata: None,
                source_message_id: user_message.id,
                created_at: None,
            })
            .expect("failed to add preserved ai context message");

        let mut context = ExecutionContext::new(session_id.to_string());
        context.state = RuntimeState::Waiting;
        context.wait_reason = Some(WaitReason::Approval);
        context.current_segment_id = 2;
        context.current_step = 7;
        context.max_steps = 42;
        context.last_action_summary = Some("waiting for approval".to_string());
        context.current_context_tokens = Some(2048);
        context.max_context_tokens = Some(202752);
        context.waiting_on_sub_agent_id = Some("sub-agent-1".to_string());
        context.sub_agent_sessions = vec!["sub-agent-1".to_string(), "sub-agent-2".to_string()];
        store
            .upsert_execution_context(&context)
            .expect("failed to persist waiting snapshot");

        assert!(store
            .delete_last_message(session_id)
            .expect("failed to delete manual clear-context marker"));

        let snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to load snapshot after deleting clear-context marker");
        assert_eq!(snapshot.messages.len(), 1);
        assert!(snapshot
            .messages
            .iter()
            .any(|message| message.id == user_message.id));
        assert!(snapshot
            .messages
            .iter()
            .all(|message| message.id != clear_marker.id));

        let rebuilt = store
            .get_execution_context(session_id)
            .expect("failed to load rebuilt snapshot")
            .expect("manual clear-context deletion should restore a snapshot");
        assert_eq!(rebuilt.state, RuntimeState::Waiting);
        assert_eq!(rebuilt.wait_reason, Some(WaitReason::Approval));
        assert_eq!(rebuilt.current_segment_id, 1);
        assert_eq!(rebuilt.current_context_tokens, Some(17753));
        assert_eq!(rebuilt.max_context_tokens, Some(202752));
        assert_eq!(rebuilt.current_step, 7);
        assert_eq!(rebuilt.max_steps, 42);
        assert_eq!(
            rebuilt.last_action_summary.as_deref(),
            Some("waiting for approval")
        );
        assert_eq!(
            rebuilt.waiting_on_sub_agent_id.as_deref(),
            Some("sub-agent-1")
        );
        assert_eq!(
            rebuilt.sub_agent_sessions,
            vec!["sub-agent-1".to_string(), "sub-agent-2".to_string()]
        );

        let conn = store
            .conn
            .lock()
            .expect("failed to lock db connection for assertions");
        let status: String = conn
            .query_row(
                "SELECT status FROM workflows WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .expect("failed to read workflow status");
        assert_eq!(status, execution_context_to_workflow_status(&rebuilt));
        let wait_reason: Option<String> = conn
            .query_row(
                "SELECT wait_reason FROM workflow_snapshots WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .expect("failed to read workflow wait_reason");
        assert_eq!(wait_reason.as_deref(), Some("approval"));
        let preserved_context_rows: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM workflow_context_messages WHERE session_id = ?1 AND segment_id = 1",
                params![session_id],
                |row| row.get(0),
            )
            .expect("failed to count preserved context rows");
        assert_eq!(preserved_context_rows, 1);
    }
}
