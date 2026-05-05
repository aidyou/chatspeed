//! Event Replay Module for Workflow Recovery
//!
//! Recovery priority: Snapshot First, Event Replay Fallback.

use crate::db::MainStore;
use crate::workflow::react::events::{WorkflowEventRecord, WorkflowEventType};
use crate::workflow::react::types::{
    ExecutionContext, PendingTool, RuntimeState, SubAgentCompletion, WaitReason,
};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RecoveryError {
    #[allow(dead_code)]
    #[error("Snapshot version mismatch: expected {expected}, got {actual}")]
    VersionMismatch { expected: String, actual: String },

    #[error("Event replay skipped: workflow has no persisted events yet")]
    EmptyReplayHistory,

    #[error("Event replay failed: {reason}")]
    ReplayFailed { reason: String },

    #[error("Missing required event data for {event_type}: {field}")]
    MissingEventData { event_type: String, field: String },

    #[allow(dead_code)]
    #[error("Invalid event sequence: {reason}")]
    InvalidSequence { reason: String },

    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::db::error::StoreError),
}

#[allow(dead_code)]
#[derive(Debug)]
pub enum RecoveryResult {
    SnapshotHit {
        context: ExecutionContext,
    },
    ReplayFallback {
        context: ExecutionContext,
    },
    SafeFailed {
        session_id: String,
        error: RecoveryError,
    },
}

#[allow(dead_code)]
impl RecoveryResult {
    pub fn is_success(&self) -> bool {
        matches!(
            self,
            RecoveryResult::SnapshotHit { .. } | RecoveryResult::ReplayFallback { .. }
        )
    }

    pub fn context(&self) -> Option<&ExecutionContext> {
        match self {
            RecoveryResult::SnapshotHit { context } => Some(context),
            RecoveryResult::ReplayFallback { context } => Some(context),
            RecoveryResult::SafeFailed { .. } => None,
        }
    }

    pub fn into_context(self) -> Option<ExecutionContext> {
        match self {
            RecoveryResult::SnapshotHit { context } => Some(context),
            RecoveryResult::ReplayFallback { context } => Some(context),
            RecoveryResult::SafeFailed { .. } => None,
        }
    }
}

impl RecoveryError {
    pub fn is_empty_replay_history(&self) -> bool {
        matches!(self, RecoveryError::EmptyReplayHistory)
    }
}

pub struct EventReducer {
    session_id: String,
    state: RuntimeState,
    wait_reason: Option<WaitReason>,
    current_step: usize,
    pending_tools: Vec<PendingTool>,
    last_action_summary: Option<String>,
    last_event_id: Option<i64>,
    waiting_on_sub_agent_id: Option<String>,
    sub_agent_sessions: Vec<String>,
    pending_sub_agent_completions: Vec<SubAgentCompletion>,
}

impl EventReducer {
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            state: RuntimeState::Pending,
            wait_reason: None,
            current_step: 0,
            pending_tools: Vec::new(),
            last_action_summary: None,
            last_event_id: None,
            waiting_on_sub_agent_id: None,
            sub_agent_sessions: Vec::new(),
            pending_sub_agent_completions: Vec::new(),
        }
    }

    pub fn apply_event(&mut self, event: &WorkflowEventRecord) -> Result<(), RecoveryError> {
        #[cfg(debug_assertions)]
        log::debug!(
            "[Workflow][session={}] replay.apply_event - type={}, id={}",
            self.session_id,
            event.event_type,
            event.id
        );

        let event_type = match event.event_type.as_str() {
            "workflow_started" => WorkflowEventType::WorkflowStarted,
            "state_changed" => WorkflowEventType::StateChanged,
            "wait_entered" => WorkflowEventType::WaitEntered,
            "user_input_received" => WorkflowEventType::UserInputReceived,
            "approval_requested" => WorkflowEventType::ApprovalRequested,
            "approval_resolved" => WorkflowEventType::ApprovalResolved,
            "tool_started" => WorkflowEventType::ToolStarted,
            "tool_completed" => WorkflowEventType::ToolCompleted,
            "tool_failed" => WorkflowEventType::ToolFailed,
            "sub_agent_started" => WorkflowEventType::SubAgentStarted,
            "sub_agent_completed" => WorkflowEventType::SubAgentCompleted,
            "sub_agent_failed" => WorkflowEventType::SubAgentFailed,
            "sub_agent_interrupted" => WorkflowEventType::SubAgentInterrupted,
            "workflow_completed" => WorkflowEventType::WorkflowCompleted,
            "workflow_failed" => WorkflowEventType::WorkflowFailed,
            "workflow_cancelled" => WorkflowEventType::WorkflowCancelled,
            _ => {
                self.last_event_id = Some(event.id);
                return Ok(());
            }
        };

        match event_type {
            WorkflowEventType::WorkflowStarted => {
                self.state = RuntimeState::Running;
                self.wait_reason = None;
            }
            WorkflowEventType::StateChanged => {
                let to_state = event.event_data["to_state"].as_str().ok_or_else(|| {
                    RecoveryError::MissingEventData {
                        event_type: "state_changed".to_string(),
                        field: "to_state".to_string(),
                    }
                })?;
                self.state = map_ui_state_to_runtime(to_state);
            }
            WorkflowEventType::WaitEntered => {
                self.state = RuntimeState::Waiting;
                let wait_reason_str =
                    event.event_data["wait_reason"].as_str().ok_or_else(|| {
                        RecoveryError::MissingEventData {
                            event_type: "wait_entered".to_string(),
                            field: "wait_reason".to_string(),
                        }
                    })?;
                self.wait_reason = parse_wait_reason(wait_reason_str);

                if let Some(tools) = event.event_data["pending_tools"].as_array() {
                    self.pending_tools.clear();
                    for tool in tools {
                        if let (Some(tool_call_id), Some(tool_name)) =
                            (tool["tool_call_id"].as_str(), tool["tool_name"].as_str())
                        {
                            self.pending_tools.push(PendingTool {
                                tool_call_id: tool_call_id.to_string(),
                                tool_name: tool_name.to_string(),
                                arguments: tool["arguments"].clone(),
                                details: tool.get("details").cloned(),
                                display_type: tool["display_type"].as_str().map(|s| s.to_string()),
                            });
                        }
                    }
                }
            }
            WorkflowEventType::UserInputReceived => {
                if self.wait_reason == Some(WaitReason::UserInput) {
                    self.wait_reason = None;
                    self.state = RuntimeState::Running;
                }
                self.current_step += 1;
            }
            WorkflowEventType::ApprovalRequested => {
                let tool_call_id = event.event_data["tool_call_id"].as_str().ok_or_else(|| {
                    RecoveryError::MissingEventData {
                        event_type: "approval_requested".to_string(),
                        field: "tool_call_id".to_string(),
                    }
                })?;
                let tool_name = event.event_data["tool_name"].as_str().ok_or_else(|| {
                    RecoveryError::MissingEventData {
                        event_type: "approval_requested".to_string(),
                        field: "tool_name".to_string(),
                    }
                })?;

                if !self
                    .pending_tools
                    .iter()
                    .any(|t| t.tool_call_id == tool_call_id)
                {
                    self.pending_tools.push(PendingTool {
                        tool_call_id: tool_call_id.to_string(),
                        tool_name: tool_name.to_string(),
                        arguments: event
                            .event_data
                            .get("arguments")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null),
                        details: event.event_data.get("details").cloned(),
                        display_type: event.event_data["display_type"]
                            .as_str()
                            .map(|s| s.to_string()),
                    });
                }
            }
            WorkflowEventType::ApprovalResolved => {
                let tool_call_id = event.event_data["tool_call_id"].as_str().ok_or_else(|| {
                    RecoveryError::MissingEventData {
                        event_type: "approval_resolved".to_string(),
                        field: "tool_call_id".to_string(),
                    }
                })?;

                self.pending_tools
                    .retain(|t| t.tool_call_id != tool_call_id);

                if self.pending_tools.is_empty() && self.wait_reason == Some(WaitReason::Approval) {
                    self.wait_reason = None;
                    self.state = RuntimeState::Running;
                }
            }
            WorkflowEventType::ToolStarted => {
                self.state = RuntimeState::Running;
                self.current_step += 1;
            }
            WorkflowEventType::ToolCompleted | WorkflowEventType::ToolFailed => {
                if let Some(result) = event.event_data.get("result") {
                    if let Some(summary) = result.as_str() {
                        self.last_action_summary =
                            Some(summary.chars().take(200).collect::<String>());
                    }
                }
            }
            WorkflowEventType::SubAgentStarted => {
                let sub_agent_id = event.event_data["sub_agent_id"].as_str().ok_or_else(|| {
                    RecoveryError::MissingEventData {
                        event_type: "sub_agent_started".to_string(),
                        field: "sub_agent_id".to_string(),
                    }
                })?;
                if !self.sub_agent_sessions.iter().any(|id| id == sub_agent_id) {
                    self.sub_agent_sessions.push(sub_agent_id.to_string());
                }
                self.waiting_on_sub_agent_id = Some(sub_agent_id.to_string());
                self.wait_reason = Some(WaitReason::SubAgent);
                self.state = RuntimeState::Waiting;
            }
            WorkflowEventType::SubAgentCompleted
            | WorkflowEventType::SubAgentFailed
            | WorkflowEventType::SubAgentInterrupted => {
                let sub_agent_id = event.event_data["sub_agent_id"].as_str().ok_or_else(|| {
                    RecoveryError::MissingEventData {
                        event_type: event.event_type.clone(),
                        field: "sub_agent_id".to_string(),
                    }
                })?;
                self.sub_agent_sessions.retain(|id| id != sub_agent_id);

                let status = event
                    .event_data
                    .get("status")
                    .and_then(|value| value.as_str())
                    .unwrap_or(match event_type {
                        WorkflowEventType::SubAgentInterrupted => "interrupted",
                        WorkflowEventType::SubAgentFailed => "failed",
                        _ => "completed",
                    })
                    .to_string();
                let result_payload = event
                    .event_data
                    .get("result")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let completion = SubAgentCompletion {
                    sub_agent_id: sub_agent_id.to_string(),
                    parent_session_id: self.session_id.clone(),
                    status: status.clone(),
                    result: result_payload
                        .get("result")
                        .and_then(|value| value.as_str())
                        .map(str::to_string),
                    summary: result_payload
                        .get("summary")
                        .and_then(|value| value.as_str())
                        .map(str::to_string),
                    error: result_payload
                        .get("error")
                        .or_else(|| event.event_data.get("reason"))
                        .and_then(|value| value.as_str())
                        .map(str::to_string),
                    tool_calls_count: result_payload
                        .get("tool_calls_count")
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0) as usize,
                    completed_at_ms: 0,
                    consumed: false,
                };
                self.pending_sub_agent_completions
                    .retain(|existing| existing.sub_agent_id != sub_agent_id);
                self.pending_sub_agent_completions.push(completion);

                if self.waiting_on_sub_agent_id.as_deref() == Some(sub_agent_id) {
                    self.wait_reason = Some(WaitReason::SubAgent);
                    self.state = RuntimeState::Waiting;
                } else {
                    self.state = RuntimeState::Running;
                }
                if let Some(summary) = event
                    .event_data
                    .get("result")
                    .and_then(|result| result.get("summary").or_else(|| result.get("error")))
                    .and_then(|value| value.as_str())
                {
                    self.last_action_summary = Some(summary.chars().take(200).collect());
                }
            }
            WorkflowEventType::WorkflowCompleted => {
                self.state = RuntimeState::Completed;
                self.wait_reason = None;
                self.pending_tools.clear();
                if let Some(summary) = event.event_data["summary"].as_str() {
                    self.last_action_summary = Some(summary.to_string());
                }
            }
            WorkflowEventType::WorkflowFailed => {
                self.state = RuntimeState::Failed;
                self.wait_reason = None;
                self.pending_tools.clear();
                if let Some(error) = event.event_data["error"].as_str() {
                    self.last_action_summary = Some(format!("Failed: {}", error));
                }
            }
            WorkflowEventType::WorkflowCancelled => {
                self.state = RuntimeState::Cancelled;
                self.wait_reason = None;
                self.pending_tools.clear();
            }
        }

        self.last_event_id = Some(event.id);
        Ok(())
    }

    pub fn build(self) -> ExecutionContext {
        ExecutionContext {
            session_id: self.session_id,
            state: self.state,
            wait_reason: self.wait_reason,
            current_step: self.current_step,
            max_steps: 100,
            pending_tools: self.pending_tools,
            last_action_summary: self.last_action_summary,
            current_context_tokens: None,
            max_context_tokens: None,
            last_event_id: self.last_event_id,
            version: ExecutionContext::CURRENT_VERSION.to_string(),
            waiting_on_sub_agent_id: self.waiting_on_sub_agent_id,
            sub_agent_sessions: self.sub_agent_sessions,
            pending_sub_agent_completions: self.pending_sub_agent_completions,
        }
    }
}

fn map_ui_state_to_runtime(ui_state: &str) -> RuntimeState {
    match ui_state {
        "pending" => RuntimeState::Pending,
        "thinking" | "executing" | "auditing" => RuntimeState::Running,
        "paused"
        | "awaiting_user"
        | "awaiting_approval"
        | "awaiting_auto_approval"
        | "awaiting_sub_agent" => RuntimeState::Waiting,
        "completed" => RuntimeState::Completed,
        "error" => RuntimeState::Failed,
        "cancelled" => RuntimeState::Cancelled,
        _ => {
            log::info!("Unknown UI state '{}' mapping to Running", ui_state);
            RuntimeState::Running
        }
    }
}

fn parse_wait_reason(s: &str) -> Option<WaitReason> {
    match s {
        "confirmation" => Some(WaitReason::Confirmation),
        "user_input" => Some(WaitReason::UserInput),
        "approval" => Some(WaitReason::Approval),
        "sub_agent" => Some(WaitReason::SubAgent),
        _ => {
            log::info!("Unknown wait reason '{}' parsing to None", s);
            None
        }
    }
}

/// Restore ExecutionContext for a session.
/// Priority: snapshot first, event replay fallback, safe-failed state on error.
pub fn restore_execution_context(
    main_store: Arc<std::sync::RwLock<MainStore>>,
    session_id: &str,
) -> RecoveryResult {
    let snapshot_result = {
        let store = match main_store.read() {
            Ok(s) => s,
            Err(e) => {
                log::error!(
                    "[Workflow][session={}] workflow.restore.failed - cannot acquire store lock: {}",
                    session_id,
                    e
                );
                return RecoveryResult::SafeFailed {
                    session_id: session_id.to_string(),
                    error: RecoveryError::DatabaseError(crate::db::error::StoreError::LockError(
                        e.to_string(),
                    )),
                };
            }
        };
        store.get_execution_context(session_id)
    };

    match snapshot_result {
        Ok(Some(ctx)) => {
            if ctx.version == ExecutionContext::CURRENT_VERSION {
                log::info!(
                    "[Workflow][session={}] workflow.restore.snapshot_hit - state={:?}, wait_reason={:?}",
                    session_id,
                    ctx.state,
                    ctx.wait_reason
                );
                RecoveryResult::SnapshotHit { context: ctx }
            } else {
                log::info!(
                    "[Workflow][session={}] workflow.restore.snapshot_version_mismatch - expected={}, got={}, falling back to replay",
                    session_id,
                    ExecutionContext::CURRENT_VERSION,
                    ctx.version
                );
                log::info!(
                    "[Workflow][session={}] workflow.restore.replay_fallback - entering event replay",
                    session_id
                );
                replay_from_events(main_store, session_id)
            }
        }
        Ok(None) => {
            log::info!(
                "[Workflow][session={}] workflow.restore.snapshot_miss - falling back to replay",
                session_id
            );
            log::info!(
                "[Workflow][session={}] workflow.restore.replay_fallback - entering event replay",
                session_id
            );
            replay_from_events(main_store, session_id)
        }
        Err(e) => {
            log::error!(
                "[Workflow][session={}] workflow.restore.snapshot_read_failed - error={}, falling back to replay",
                session_id,
                e
            );
            log::info!(
                "[Workflow][session={}] workflow.restore.replay_fallback - entering event replay",
                session_id
            );
            replay_from_events(main_store, session_id)
        }
    }
}

fn replay_from_events(
    main_store: Arc<std::sync::RwLock<MainStore>>,
    session_id: &str,
) -> RecoveryResult {
    log::info!(
        "[Workflow][session={}] workflow.replay.start - beginning event replay",
        session_id
    );

    let events = {
        let store = match main_store.read() {
            Ok(s) => s,
            Err(e) => {
                log::error!(
                    "[Workflow][session={}] workflow.replay.failed - cannot acquire store lock: {}",
                    session_id,
                    e
                );
                return RecoveryResult::SafeFailed {
                    session_id: session_id.to_string(),
                    error: RecoveryError::DatabaseError(crate::db::error::StoreError::LockError(
                        e.to_string(),
                    )),
                };
            }
        };

        match store.list_workflow_events(session_id) {
            Ok(events) => events,
            Err(e) => {
                log::error!(
                    "[Workflow][session={}] workflow.replay.failed - cannot load events: {}",
                    session_id,
                    e
                );
                return RecoveryResult::SafeFailed {
                    session_id: session_id.to_string(),
                    error: RecoveryError::ReplayFailed {
                        reason: format!("Cannot load events: {}", e),
                    },
                };
            }
        }
    };

    if events.is_empty() {
        log::info!(
            "[Workflow][session={}] workflow.replay.no_events - no events found for replay yet; treating as empty workflow history",
            session_id
        );
        return RecoveryResult::SafeFailed {
            session_id: session_id.to_string(),
            error: RecoveryError::EmptyReplayHistory,
        };
    }

    let mut reducer = EventReducer::new(session_id.to_string());
    for event in &events {
        if let Err(e) = reducer.apply_event(event) {
            log::error!(
                "[Workflow][session={}] workflow.replay.failed - error applying event {}: {}",
                session_id,
                event.id,
                e
            );
            return RecoveryResult::SafeFailed {
                session_id: session_id.to_string(),
                error: e,
            };
        }
    }

    let context = reducer.build();

    log::info!(
        "[Workflow][session={}] workflow.replay.done - state={:?}, wait_reason={:?}, pending_tools={}, last_event_id={:?}",
        session_id,
        context.state,
        context.wait_reason,
        context.pending_tools.len(),
        context.last_event_id
    );

    RecoveryResult::ReplayFallback { context }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reducer_workflow_started() {
        let mut reducer = EventReducer::new("test-session".to_string());
        let event = WorkflowEventRecord {
            id: 1,
            session_id: "test-session".to_string(),
            event_type: "workflow_started".to_string(),
            event_version: "1.0.0".to_string(),
            event_data: serde_json::json!({ "agent_id": "agent-001" }),
            created_at: "2026-01-01 00:00:00".to_string(),
        };

        reducer.apply_event(&event).unwrap();
        let ctx = reducer.build();

        assert_eq!(ctx.state, RuntimeState::Running);
        assert!(ctx.wait_reason.is_none());
        assert_eq!(ctx.last_event_id, Some(1));
    }

    #[test]
    fn test_reducer_wait_entered_approval() {
        let mut reducer = EventReducer::new("test-session".to_string());

        let started = WorkflowEventRecord {
            id: 1,
            session_id: "test-session".to_string(),
            event_type: "workflow_started".to_string(),
            event_version: "1.0.0".to_string(),
            event_data: serde_json::json!({ "agent_id": "agent-001" }),
            created_at: "2026-01-01 00:00:00".to_string(),
        };
        reducer.apply_event(&started).unwrap();

        let wait_event = WorkflowEventRecord {
            id: 2,
            session_id: "test-session".to_string(),
            event_type: "wait_entered".to_string(),
            event_version: "1.0.0".to_string(),
            event_data: serde_json::json!({
                "wait_reason": "approval",
                "pending_tools": [
                    {
                        "tool_call_id": "call_123",
                        "tool_name": "bash",
                        "arguments": {"command": "ls"},
                        "details": "List files"
                    }
                ]
            }),
            created_at: "2026-01-01 00:00:01".to_string(),
        };
        reducer.apply_event(&wait_event).unwrap();

        let ctx = reducer.build();

        assert_eq!(ctx.state, RuntimeState::Waiting);
        assert_eq!(ctx.wait_reason, Some(WaitReason::Approval));
        assert_eq!(ctx.pending_tools.len(), 1);
        assert_eq!(ctx.pending_tools[0].tool_call_id, "call_123");
        assert_eq!(ctx.pending_tools[0].tool_name, "bash");
    }

    #[test]
    fn test_reducer_approval_resolved() {
        let mut reducer = EventReducer::new("test-session".to_string());

        let started = WorkflowEventRecord {
            id: 1,
            session_id: "test-session".to_string(),
            event_type: "workflow_started".to_string(),
            event_version: "1.0.0".to_string(),
            event_data: serde_json::json!({ "agent_id": "agent-001" }),
            created_at: "2026-01-01 00:00:00".to_string(),
        };
        reducer.apply_event(&started).unwrap();

        let wait = WorkflowEventRecord {
            id: 2,
            session_id: "test-session".to_string(),
            event_type: "wait_entered".to_string(),
            event_version: "1.0.0".to_string(),
            event_data: serde_json::json!({
                "wait_reason": "approval",
                "pending_tools": [{"tool_call_id": "call_123", "tool_name": "bash"}]
            }),
            created_at: "2026-01-01 00:00:01".to_string(),
        };
        reducer.apply_event(&wait).unwrap();

        let resolved = WorkflowEventRecord {
            id: 3,
            session_id: "test-session".to_string(),
            event_type: "approval_resolved".to_string(),
            event_version: "1.0.0".to_string(),
            event_data: serde_json::json!({
                "tool_call_id": "call_123",
                "approved": true,
                "approve_all": false
            }),
            created_at: "2026-01-01 00:00:02".to_string(),
        };
        reducer.apply_event(&resolved).unwrap();

        let ctx = reducer.build();

        assert_eq!(ctx.state, RuntimeState::Running);
        assert!(ctx.wait_reason.is_none());
        assert!(ctx.pending_tools.is_empty());
    }

    #[test]
    fn test_reducer_workflow_completed() {
        let mut reducer = EventReducer::new("test-session".to_string());

        let started = WorkflowEventRecord {
            id: 1,
            session_id: "test-session".to_string(),
            event_type: "workflow_started".to_string(),
            event_version: "1.0.0".to_string(),
            event_data: serde_json::json!({ "agent_id": "agent-001" }),
            created_at: "2026-01-01 00:00:00".to_string(),
        };
        reducer.apply_event(&started).unwrap();

        let completed = WorkflowEventRecord {
            id: 2,
            session_id: "test-session".to_string(),
            event_type: "workflow_completed".to_string(),
            event_version: "1.0.0".to_string(),
            event_data: serde_json::json!({ "summary": "Task completed successfully" }),
            created_at: "2026-01-01 00:00:10".to_string(),
        };
        reducer.apply_event(&completed).unwrap();

        let ctx = reducer.build();

        assert_eq!(ctx.state, RuntimeState::Completed);
        assert!(ctx.wait_reason.is_none());
        assert_eq!(
            ctx.last_action_summary,
            Some("Task completed successfully".to_string())
        );
    }

    #[test]
    fn test_reducer_workflow_failed() {
        let mut reducer = EventReducer::new("test-session".to_string());

        let started = WorkflowEventRecord {
            id: 1,
            session_id: "test-session".to_string(),
            event_type: "workflow_started".to_string(),
            event_version: "1.0.0".to_string(),
            event_data: serde_json::json!({ "agent_id": "agent-001" }),
            created_at: "2026-01-01 00:00:00".to_string(),
        };
        reducer.apply_event(&started).unwrap();

        let failed = WorkflowEventRecord {
            id: 2,
            session_id: "test-session".to_string(),
            event_type: "workflow_failed".to_string(),
            event_version: "1.0.0".to_string(),
            event_data: serde_json::json!({ "error": "Connection timeout" }),
            created_at: "2026-01-01 00:00:10".to_string(),
        };
        reducer.apply_event(&failed).unwrap();

        let ctx = reducer.build();

        assert_eq!(ctx.state, RuntimeState::Failed);
        assert_eq!(
            ctx.last_action_summary,
            Some("Failed: Connection timeout".to_string())
        );
    }

    #[test]
    fn test_reducer_workflow_cancelled() {
        let mut reducer = EventReducer::new("test-session".to_string());

        let started = WorkflowEventRecord {
            id: 1,
            session_id: "test-session".to_string(),
            event_type: "workflow_started".to_string(),
            event_version: "1.0.0".to_string(),
            event_data: serde_json::json!({ "agent_id": "agent-001" }),
            created_at: "2026-01-01 00:00:00".to_string(),
        };
        reducer.apply_event(&started).unwrap();

        let cancelled = WorkflowEventRecord {
            id: 2,
            session_id: "test-session".to_string(),
            event_type: "workflow_cancelled".to_string(),
            event_version: "1.0.0".to_string(),
            event_data: serde_json::json!({}),
            created_at: "2026-01-01 00:00:10".to_string(),
        };
        reducer.apply_event(&cancelled).unwrap();

        let ctx = reducer.build();

        assert_eq!(ctx.state, RuntimeState::Cancelled);
        assert!(ctx.wait_reason.is_none());
    }

    #[test]
    fn test_reducer_multiple_pending_tools() {
        let mut reducer = EventReducer::new("test-session".to_string());

        let wait = WorkflowEventRecord {
            id: 1,
            session_id: "test-session".to_string(),
            event_type: "wait_entered".to_string(),
            event_version: "1.0.0".to_string(),
            event_data: serde_json::json!({
                "wait_reason": "approval",
                "pending_tools": [
                    {"tool_call_id": "call_1", "tool_name": "bash", "arguments": {}},
                    {"tool_call_id": "call_2", "tool_name": "write_file", "arguments": {}},
                    {"tool_call_id": "call_3", "tool_name": "read_file", "arguments": {}}
                ]
            }),
            created_at: "2026-01-01 00:00:00".to_string(),
        };
        reducer.apply_event(&wait).unwrap();

        let ctx = reducer.build();

        assert_eq!(ctx.pending_tools.len(), 3);
        assert_eq!(ctx.pending_tools[0].tool_call_id, "call_1");
        assert_eq!(ctx.pending_tools[1].tool_call_id, "call_2");
        assert_eq!(ctx.pending_tools[2].tool_call_id, "call_3");
    }

    #[test]
    fn test_reducer_user_input_received() {
        let mut reducer = EventReducer::new("test-session".to_string());

        let wait = WorkflowEventRecord {
            id: 1,
            session_id: "test-session".to_string(),
            event_type: "wait_entered".to_string(),
            event_version: "1.0.0".to_string(),
            event_data: serde_json::json!({ "wait_reason": "user_input" }),
            created_at: "2026-01-01 00:00:00".to_string(),
        };
        reducer.apply_event(&wait).unwrap();

        assert_eq!(reducer.state, RuntimeState::Waiting);
        assert_eq!(reducer.wait_reason, Some(WaitReason::UserInput));

        let input = WorkflowEventRecord {
            id: 2,
            session_id: "test-session".to_string(),
            event_type: "user_input_received".to_string(),
            event_version: "1.0.0".to_string(),
            event_data: serde_json::json!({ "content": "Please continue" }),
            created_at: "2026-01-01 00:00:05".to_string(),
        };
        reducer.apply_event(&input).unwrap();

        let ctx = reducer.build();

        assert_eq!(ctx.state, RuntimeState::Running);
        assert!(ctx.wait_reason.is_none());
    }

    #[test]
    fn test_reducer_unknown_event_type_skipped() {
        let mut reducer = EventReducer::new("test-session".to_string());

        let unknown = WorkflowEventRecord {
            id: 1,
            session_id: "test-session".to_string(),
            event_type: "custom_event".to_string(),
            event_version: "1.0.0".to_string(),
            event_data: serde_json::json!({}),
            created_at: "2026-01-01 00:00:00".to_string(),
        };

        let result = reducer.apply_event(&unknown);
        assert!(result.is_ok());

        let ctx = reducer.build();
        assert_eq!(ctx.last_event_id, Some(1));
    }

    #[test]
    fn test_map_ui_state_to_runtime() {
        assert_eq!(map_ui_state_to_runtime("pending"), RuntimeState::Pending);
        assert_eq!(map_ui_state_to_runtime("thinking"), RuntimeState::Running);
        assert_eq!(map_ui_state_to_runtime("executing"), RuntimeState::Running);
        assert_eq!(
            map_ui_state_to_runtime("awaiting_user"),
            RuntimeState::Waiting
        );
        assert_eq!(
            map_ui_state_to_runtime("awaiting_approval"),
            RuntimeState::Waiting
        );
        assert_eq!(
            map_ui_state_to_runtime("awaiting_sub_agent"),
            RuntimeState::Waiting
        );
        assert_eq!(
            map_ui_state_to_runtime("completed"),
            RuntimeState::Completed
        );
        assert_eq!(map_ui_state_to_runtime("error"), RuntimeState::Failed);
        assert_eq!(
            map_ui_state_to_runtime("cancelled"),
            RuntimeState::Cancelled
        );
    }

    #[test]
    fn test_parse_wait_reason() {
        assert_eq!(
            parse_wait_reason("confirmation"),
            Some(WaitReason::Confirmation)
        );
        assert_eq!(parse_wait_reason("user_input"), Some(WaitReason::UserInput));
        assert_eq!(parse_wait_reason("approval"), Some(WaitReason::Approval));
        assert_eq!(parse_wait_reason("sub_agent"), Some(WaitReason::SubAgent));
        assert_eq!(parse_wait_reason("unknown"), None);
    }

    #[test]
    fn test_recovery_result_is_success() {
        let ctx = ExecutionContext::new("test".to_string());

        let snapshot_hit = RecoveryResult::SnapshotHit {
            context: ctx.clone(),
        };
        assert!(snapshot_hit.is_success());

        let replay_fallback = RecoveryResult::ReplayFallback { context: ctx };
        assert!(replay_fallback.is_success());

        let safe_failed = RecoveryResult::SafeFailed {
            session_id: "test".to_string(),
            error: RecoveryError::ReplayFailed {
                reason: "test".to_string(),
            },
        };
        assert!(!safe_failed.is_success());
    }

    #[test]
    fn test_recovery_result_context() {
        let ctx = ExecutionContext::new("test".to_string());

        let snapshot_hit = RecoveryResult::SnapshotHit {
            context: ctx.clone(),
        };
        assert!(snapshot_hit.context().is_some());

        let safe_failed = RecoveryResult::SafeFailed {
            session_id: "test".to_string(),
            error: RecoveryError::ReplayFailed {
                reason: "test".to_string(),
            },
        };
        assert!(safe_failed.context().is_none());
    }

    #[test]
    fn test_recovery_result_into_context() {
        let ctx = ExecutionContext::new("test".to_string());

        let snapshot_hit = RecoveryResult::SnapshotHit {
            context: ctx.clone(),
        };
        let extracted = snapshot_hit.into_context();
        assert!(extracted.is_some());
        assert_eq!(extracted.unwrap().session_id, "test");

        let safe_failed = RecoveryResult::SafeFailed {
            session_id: "test".to_string(),
            error: RecoveryError::ReplayFailed {
                reason: "test".to_string(),
            },
        };
        assert!(safe_failed.into_context().is_none());
    }

    #[test]
    fn test_reducer_complex_approval_flow() {
        let mut reducer = EventReducer::new("complex-session".to_string());

        let events = vec![
            WorkflowEventRecord {
                id: 1,
                session_id: "complex-session".to_string(),
                event_type: "workflow_started".to_string(),
                event_version: "1.0.0".to_string(),
                event_data: serde_json::json!({ "agent_id": "agent-001" }),
                created_at: "2026-01-01 00:00:00".to_string(),
            },
            WorkflowEventRecord {
                id: 2,
                session_id: "complex-session".to_string(),
                event_type: "state_changed".to_string(),
                event_version: "1.0.0".to_string(),
                event_data: serde_json::json!({ "from_state": "pending", "to_state": "thinking" }),
                created_at: "2026-01-01 00:00:01".to_string(),
            },
            WorkflowEventRecord {
                id: 3,
                session_id: "complex-session".to_string(),
                event_type: "tool_started".to_string(),
                event_version: "1.0.0".to_string(),
                event_data: serde_json::json!({ "tool_call_id": "call_1", "tool_name": "bash" }),
                created_at: "2026-01-01 00:00:02".to_string(),
            },
            WorkflowEventRecord {
                id: 4,
                session_id: "complex-session".to_string(),
                event_type: "approval_requested".to_string(),
                event_version: "1.0.0".to_string(),
                event_data: serde_json::json!({ "tool_call_id": "call_2", "tool_name": "write_file" }),
                created_at: "2026-01-01 00:00:03".to_string(),
            },
            WorkflowEventRecord {
                id: 5,
                session_id: "complex-session".to_string(),
                event_type: "wait_entered".to_string(),
                event_version: "1.0.0".to_string(),
                event_data: serde_json::json!({
                    "wait_reason": "approval",
                    "pending_tools": [
                        {"tool_call_id": "call_2", "tool_name": "write_file", "arguments": {}}
                    ]
                }),
                created_at: "2026-01-01 00:00:04".to_string(),
            },
            WorkflowEventRecord {
                id: 6,
                session_id: "complex-session".to_string(),
                event_type: "approval_resolved".to_string(),
                event_version: "1.0.0".to_string(),
                event_data: serde_json::json!({ "tool_call_id": "call_2", "approved": true, "approve_all": false }),
                created_at: "2026-01-01 00:00:05".to_string(),
            },
            WorkflowEventRecord {
                id: 7,
                session_id: "complex-session".to_string(),
                event_type: "workflow_completed".to_string(),
                event_version: "1.0.0".to_string(),
                event_data: serde_json::json!({ "summary": "Done" }),
                created_at: "2026-01-01 00:00:10".to_string(),
            },
        ];

        for event in &events {
            reducer.apply_event(event).unwrap();
        }

        let ctx = reducer.build();

        assert_eq!(ctx.state, RuntimeState::Completed);
        assert!(ctx.wait_reason.is_none());
        assert!(ctx.pending_tools.is_empty());
        assert_eq!(ctx.current_step, 1);
        assert_eq!(ctx.last_event_id, Some(7));
    }

    #[test]
    fn test_reducer_keeps_pending_sub_agent_completion_for_restore() {
        let mut reducer = EventReducer::new("test-session".to_string());

        let events = vec![
            WorkflowEventRecord {
                id: 1,
                session_id: "test-session".to_string(),
                event_type: "sub_agent_started".to_string(),
                event_version: "1.0.0".to_string(),
                event_data: serde_json::json!({
                    "sub_agent_id": "subagent_1",
                    "execution_mode": "call"
                }),
                created_at: "2026-01-01 00:00:00".to_string(),
            },
            WorkflowEventRecord {
                id: 2,
                session_id: "test-session".to_string(),
                event_type: "sub_agent_completed".to_string(),
                event_version: "1.0.0".to_string(),
                event_data: serde_json::json!({
                    "sub_agent_id": "subagent_1",
                    "status": "completed",
                    "result": {
                        "result": "analysis done",
                        "summary": "done",
                        "tool_calls_count": 3
                    }
                }),
                created_at: "2026-01-01 00:00:01".to_string(),
            },
        ];

        for event in &events {
            reducer.apply_event(event).unwrap();
        }

        let ctx = reducer.build();
        assert_eq!(ctx.state, RuntimeState::Waiting);
        assert_eq!(ctx.wait_reason, Some(WaitReason::SubAgent));
        assert_eq!(ctx.waiting_on_sub_agent_id.as_deref(), Some("subagent_1"));
        assert_eq!(ctx.pending_sub_agent_completions.len(), 1);
        assert_eq!(
            ctx.pending_sub_agent_completions[0].result.as_deref(),
            Some("analysis done")
        );
    }

    #[test]
    fn test_reducer_error_handling_missing_field() {
        let mut reducer = EventReducer::new("test".to_string());

        let bad_event = WorkflowEventRecord {
            id: 1,
            session_id: "test".to_string(),
            event_type: "state_changed".to_string(),
            event_version: "1.0.0".to_string(),
            event_data: serde_json::json!({ "from_state": "pending" }),
            created_at: "2026-01-01 00:00:00".to_string(),
        };

        let result = reducer.apply_event(&bad_event);
        assert!(result.is_err());

        match result {
            Err(RecoveryError::MissingEventData { event_type, field }) => {
                assert_eq!(event_type, "state_changed");
                assert_eq!(field, "to_state");
            }
            _ => panic!("Expected MissingEventData error"),
        }
    }

    // ==================== Integration Tests with Database ====================

    mod integration {
        use super::*;
        use crate::db::MainStore;
        use crate::workflow::react::events::WorkflowEvent;
        use std::sync::{Arc, RwLock};
        use tempfile::tempdir;

        fn create_test_store() -> Arc<RwLock<MainStore>> {
            let dir = tempdir().expect("failed to create temp dir");
            let db_path = dir.path().join("replay_integration_test.db");
            let store = MainStore::new(db_path).expect("failed to create MainStore");
            Arc::new(RwLock::new(store))
        }

        #[test]
        fn test_restore_snapshot_hit() {
            let store = create_test_store();
            let session_id = "snapshot-hit-session";

            // Write snapshot
            let mut ctx = ExecutionContext::new(session_id.to_string());
            ctx.state = RuntimeState::Waiting;
            ctx.wait_reason = Some(WaitReason::Approval);
            ctx.pending_tools.push(PendingTool {
                tool_call_id: "call_1".to_string(),
                tool_name: "bash".to_string(),
                arguments: serde_json::json!({"command": "ls"}),
                details: Some(serde_json::json!("List files")),
                display_type: Some("text".to_string()),
            });

            {
                let s = store.read().unwrap();
                s.upsert_execution_context(&ctx).unwrap();
            }

            // Write some events too (should be ignored when snapshot exists)
            {
                let s = store.read().unwrap();
                let e1 =
                    WorkflowEvent::workflow_started(session_id.to_string(), "agent".to_string());
                s.append_workflow_event(&e1).unwrap();
            }

            // Restore
            let result = restore_execution_context(store.clone(), session_id);

            match result {
                RecoveryResult::SnapshotHit { context } => {
                    assert_eq!(context.state, RuntimeState::Waiting);
                    assert_eq!(context.wait_reason, Some(WaitReason::Approval));
                    assert_eq!(context.pending_tools.len(), 1);
                }
                _ => panic!("Expected SnapshotHit, got {:?}", result),
            }
        }

        #[test]
        fn test_restore_snapshot_miss_fallback_to_replay() {
            let store = create_test_store();
            let session_id = "snapshot-miss-session";

            // Don't write snapshot, only write events
            {
                let s = store.read().unwrap();
                let e1 =
                    WorkflowEvent::workflow_started(session_id.to_string(), "agent".to_string());
                s.append_workflow_event(&e1).unwrap();

                let e2 = WorkflowEvent::state_changed(
                    session_id.to_string(),
                    "pending".to_string(),
                    "awaiting_approval".to_string(),
                );
                s.append_workflow_event(&e2).unwrap();

                let e3 = WorkflowEvent::wait_entered(
                    session_id.to_string(),
                    "approval".to_string(),
                    vec![serde_json::json!({
                        "tool_call_id": "call_123",
                        "tool_name": "write_file"
                    })],
                );
                s.append_workflow_event(&e3).unwrap();
            }

            // Restore
            let result = restore_execution_context(store.clone(), session_id);

            match result {
                RecoveryResult::ReplayFallback { context } => {
                    assert_eq!(context.state, RuntimeState::Waiting);
                    assert_eq!(context.wait_reason, Some(WaitReason::Approval));
                    assert_eq!(context.pending_tools.len(), 1);
                    assert_eq!(context.pending_tools[0].tool_call_id, "call_123");
                }
                _ => panic!("Expected ReplayFallback, got {:?}", result),
            }
        }

        #[test]
        fn test_restore_version_mismatch_fallback_to_replay() {
            let store = create_test_store();
            let session_id = "version-mismatch-session";

            // Write snapshot with old version
            let mut ctx = ExecutionContext::new(session_id.to_string());
            ctx.state = RuntimeState::Running;
            ctx.version = "0.9.0".to_string(); // Old version

            {
                let s = store.read().unwrap();
                s.upsert_execution_context(&ctx).unwrap();
            }

            // Write events that represent the actual state
            {
                let s = store.read().unwrap();
                let e1 =
                    WorkflowEvent::workflow_started(session_id.to_string(), "agent".to_string());
                s.append_workflow_event(&e1).unwrap();

                let e2 = WorkflowEvent::wait_entered(
                    session_id.to_string(),
                    "user_input".to_string(),
                    vec![],
                );
                s.append_workflow_event(&e2).unwrap();
            }

            // Restore
            let result = restore_execution_context(store.clone(), session_id);

            match result {
                RecoveryResult::ReplayFallback { context } => {
                    assert_eq!(context.state, RuntimeState::Waiting);
                    assert_eq!(context.wait_reason, Some(WaitReason::UserInput));
                }
                _ => panic!("Expected ReplayFallback, got {:?}", result),
            }
        }

        #[test]
        fn test_restore_replay_failed_safe_state() {
            let store = create_test_store();
            let session_id = "replay-failed-session";

            // Write malformed events directly to DB (missing required data)
            {
                let s = store.read().unwrap();
                let conn = s.conn.lock().unwrap();
                conn.execute(
                    "INSERT INTO workflow_events (session_id, event_type, event_version, event_data, created_at)
                     VALUES (?1, ?2, ?3, ?4, datetime('now'))",
                    rusqlite::params![
                        session_id,
                        "state_changed",
                        "1.0.0",
                        r#"{"from_state": "pending"}"#
                    ],
                ).unwrap();
            }

            // Restore should fail safely
            let result = restore_execution_context(store.clone(), session_id);

            match result {
                RecoveryResult::SafeFailed {
                    session_id: sid,
                    error,
                } => {
                    assert_eq!(sid, session_id);
                    match error {
                        RecoveryError::MissingEventData { .. } => {}
                        _ => panic!("Expected MissingEventData error"),
                    }
                }
                _ => panic!("Expected SafeFailed, got {:?}", result),
            }
        }

        #[test]
        fn test_restore_no_events_safe_state() {
            let store = create_test_store();
            let session_id = "no-events-session";

            // Don't write snapshot or events
            let result = restore_execution_context(store.clone(), session_id);

            match result {
                RecoveryResult::SafeFailed {
                    session_id: sid,
                    error,
                } => {
                    assert_eq!(sid, session_id);
                    match error {
                        RecoveryError::EmptyReplayHistory => {}
                        _ => panic!("Expected EmptyReplayHistory error"),
                    }
                }
                _ => panic!("Expected SafeFailed, got {:?}", result),
            }
        }

        #[test]
        fn test_restore_terminal_states() {
            let store = create_test_store();
            let session_id = "terminal-session";

            // Write events ending in completed
            {
                let s = store.read().unwrap();
                let e1 =
                    WorkflowEvent::workflow_started(session_id.to_string(), "agent".to_string());
                s.append_workflow_event(&e1).unwrap();

                let e2 = WorkflowEvent::workflow_completed(
                    session_id.to_string(),
                    Some("Task done".to_string()),
                );
                s.append_workflow_event(&e2).unwrap();
            }

            let result = restore_execution_context(store.clone(), session_id);

            match result {
                RecoveryResult::ReplayFallback { context } => {
                    assert_eq!(context.state, RuntimeState::Completed);
                    assert!(context.wait_reason.is_none());
                    assert!(context.pending_tools.is_empty());
                }
                _ => panic!("Expected ReplayFallback, got {:?}", result),
            }
        }

        #[test]
        fn test_restore_cancelled_state() {
            let store = create_test_store();
            let session_id = "cancelled-session";

            {
                let s = store.read().unwrap();
                let e1 =
                    WorkflowEvent::workflow_started(session_id.to_string(), "agent".to_string());
                s.append_workflow_event(&e1).unwrap();

                let e2 = WorkflowEvent::workflow_cancelled(session_id.to_string());
                s.append_workflow_event(&e2).unwrap();
            }

            let result = restore_execution_context(store.clone(), session_id);

            match result {
                RecoveryResult::ReplayFallback { context } => {
                    assert_eq!(context.state, RuntimeState::Cancelled);
                    assert!(context.wait_reason.is_none());
                }
                _ => panic!("Expected ReplayFallback, got {:?}", result),
            }
        }

        #[test]
        fn test_restore_failed_state() {
            let store = create_test_store();
            let session_id = "failed-session";

            {
                let s = store.read().unwrap();
                let e1 =
                    WorkflowEvent::workflow_started(session_id.to_string(), "agent".to_string());
                s.append_workflow_event(&e1).unwrap();

                let e2 = WorkflowEvent::workflow_failed(
                    session_id.to_string(),
                    "Something went wrong".to_string(),
                );
                s.append_workflow_event(&e2).unwrap();
            }

            let result = restore_execution_context(store.clone(), session_id);

            match result {
                RecoveryResult::ReplayFallback { context } => {
                    assert_eq!(context.state, RuntimeState::Failed);
                    assert_eq!(
                        context.last_action_summary,
                        Some("Failed: Something went wrong".to_string())
                    );
                }
                _ => panic!("Expected ReplayFallback, got {:?}", result),
            }
        }

        #[test]
        fn test_reducer_field_completeness() {
            let store = create_test_store();
            let session_id = "field-complete-session";

            // Write a complex event chain
            {
                let s = store.read().unwrap();
                let e1 = WorkflowEvent::workflow_started(
                    session_id.to_string(),
                    "agent-001".to_string(),
                );
                s.append_workflow_event(&e1).unwrap();

                let e2 = WorkflowEvent::state_changed(
                    session_id.to_string(),
                    "pending".to_string(),
                    "thinking".to_string(),
                );
                s.append_workflow_event(&e2).unwrap();

                let e3 = WorkflowEvent::tool_started(
                    session_id.to_string(),
                    "call_1".to_string(),
                    "bash".to_string(),
                    serde_json::json!({"command": "ls"}),
                );
                s.append_workflow_event(&e3).unwrap();

                let e4 = WorkflowEvent::approval_requested(
                    session_id.to_string(),
                    "call_2".to_string(),
                    "write_file".to_string(),
                    serde_json::json!({"path": "/tmp/test.txt", "content": "hello"}),
                    Some(serde_json::json!("Write test file")),
                    Some("diff".to_string()),
                );
                s.append_workflow_event(&e4).unwrap();

                let e5 = WorkflowEvent::wait_entered(
                    session_id.to_string(),
                    "approval".to_string(),
                    vec![serde_json::json!({
                        "tool_call_id": "call_2",
                        "tool_name": "write_file",
                        "arguments": {"path": "/tmp/test.txt"},
                        "details": "Write test file"
                    })],
                );
                s.append_workflow_event(&e5).unwrap();
            }

            let result = restore_execution_context(store.clone(), session_id);

            match result {
                RecoveryResult::ReplayFallback { context } => {
                    // Verify all fields
                    assert_eq!(context.session_id, session_id);
                    assert_eq!(context.state, RuntimeState::Waiting);
                    assert_eq!(context.wait_reason, Some(WaitReason::Approval));
                    assert_eq!(context.current_step, 1); // One tool_started
                    assert_eq!(context.max_steps, 100);
                    assert_eq!(context.pending_tools.len(), 1);
                    assert_eq!(context.pending_tools[0].tool_call_id, "call_2");
                    assert_eq!(context.pending_tools[0].tool_name, "write_file");
                    assert_eq!(
                        context.pending_tools[0].arguments,
                        serde_json::json!({"path": "/tmp/test.txt"})
                    );
                    assert_eq!(
                        context.pending_tools[0].details,
                        Some(serde_json::json!("Write test file"))
                    );
                    assert_eq!(context.version, ExecutionContext::CURRENT_VERSION);
                    assert!(context.last_event_id.is_some());
                }
                _ => panic!("Expected ReplayFallback, got {:?}", result),
            }
        }
    }
}
