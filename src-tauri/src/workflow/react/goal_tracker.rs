//! Runtime goal tracking at blocking compression boundaries.
//!
//! The tracker turns the compression window (the user directives and completed-work
//! summaries since the previous checkpoint) into the ordered goal structure the handoff
//! carries. It is a runtime-owned step: the compression model no longer reconstructs the
//! current goal or an execution-requirement list from the raw directive history.

use crate::ai::chat::openai::OpenAIChat;
use crate::ai::interaction::chat_completion::{AiChatEnum, ChatState};
use crate::ai::traits::chat::{ChatMetadata, WorkflowUsageAttribution};
use crate::db::WorkflowMessage;
use crate::libs::util::format_json_str;
use crate::workflow::react::context::ContextManager;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::prompts::GOAL_TRACKING_PROMPT;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

/// Largest number of user directives handed to a single tracking call. A longer window is
/// split so the tracker never has to segment an unbounded burst in one reply.
const MAX_USERS_PER_CALL: usize = 15;

/// Total tracking attempts per chunk: the first attempt plus four retries with exponential
/// backoff. Exhausting them is a fatal tracking failure, not a silent skip.
const MAX_TRACKING_ATTEMPTS: u32 = 5;

/// Output budget for one tracking reply. Reasoning models spend part of the same budget on
/// hidden reasoning tokens, so a small cap truncates the JSON.
const TRACKING_MAX_OUTPUT_TOKENS: u32 = 8_192;

/// Bounded text kept from one window item before it is shown to the tracker.
const WINDOW_ITEM_CHAR_LIMIT: usize = 700;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrackedGoalStatus {
    Active,
    Completed,
    Dormant,
}

impl TrackedGoalStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Dormant => "dormant",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "active" => Some(Self::Active),
            "completed" | "complete" => Some(Self::Completed),
            "dormant" => Some(Self::Dormant),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrackedGoal {
    pub goal_id: String,
    pub summary: String,
    pub status: TrackedGoalStatus,
}

/// One ordered entry of the compression window handed to the tracker.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GoalWindowItem {
    User { text: String },
    CompletedWork { summary: String },
    Restart,
}

/// Runtime goal tracker for blocking compression boundaries.
pub(crate) struct GoalTracker {
    pub chat_state: Arc<ChatState>,
    pub provider_id: i64,
    pub model: String,
    pub workflow_usage_attribution: WorkflowUsageAttribution,
}

impl GoalTracker {
    pub(crate) fn new(
        chat_state: Arc<ChatState>,
        provider_id: i64,
        model: String,
        workflow_usage_attribution: WorkflowUsageAttribution,
    ) -> Self {
        Self {
            chat_state,
            provider_id,
            model,
            workflow_usage_attribution,
        }
    }

    /// Tracks the window and returns the current goal the handoff should carry.
    ///
    /// Returns `Ok(None)` when the window holds no user directive, so a boundary with nothing
    /// to track does not invent a goal.
    pub(crate) async fn track_current_goal(
        &self,
        window: &[WorkflowMessage],
    ) -> Result<Option<String>, WorkflowEngineError> {
        let items = Self::window_items(window);
        if !items
            .iter()
            .any(|item| matches!(item, GoalWindowItem::User { .. }))
        {
            return Ok(None);
        }
        let mut carry: Vec<TrackedGoal> = Vec::new();
        let mut next_goal_index = 1usize;
        for chunk in Self::chunk_items(&items, MAX_USERS_PER_CALL) {
            let goals = self
                .track_chunk(&chunk, &carry, &mut next_goal_index)
                .await?;
            carry = goals;
        }
        Ok(Self::current_goal(&carry))
    }

    /// The goal the handoff carries: the last still-active goal, or the last tracked record so
    /// the goal list is never empty.
    fn current_goal(goals: &[TrackedGoal]) -> Option<String> {
        goals
            .iter()
            .rev()
            .find(|goal| goal.status == TrackedGoalStatus::Active)
            .or_else(|| goals.last())
            .map(|goal| goal.summary.clone())
            .filter(|summary| !summary.trim().is_empty())
    }

    /// Keeps user directives, completed-work summaries, and clear-context markers in order.
    /// Runtime observations, approved plans, reviews, and answers to `ask_user` are excluded.
    fn window_items(window: &[WorkflowMessage]) -> Vec<GoalWindowItem> {
        let mut items = Vec::new();
        for message in window {
            if ContextManager::is_manual_clear_context_message(message) {
                items.push(GoalWindowItem::Restart);
                continue;
            }
            if ContextManager::is_successful_completion_message(message) {
                let summary = Self::completion_summary(message);
                if !summary.is_empty() {
                    items.push(GoalWindowItem::CompletedWork { summary });
                }
                continue;
            }
            if !ContextManager::is_effective_task_objective_directive(message) {
                continue;
            }
            let text = ContextManager::strip_system_reminder_blocks(&message.message);
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            items.push(GoalWindowItem::User {
                text: Self::clip(text, WINDOW_ITEM_CHAR_LIMIT),
            });
        }
        items
    }

    fn completion_summary(message: &WorkflowMessage) -> String {
        let summary = message
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("summary"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|summary| !summary.is_empty())
            .map(str::to_string);
        summary.unwrap_or_else(|| Self::clip(message.message.trim(), WINDOW_ITEM_CHAR_LIMIT))
    }

    /// Splits the window so one call never has to segment more than `max_users` directives.
    /// A restart marker stays inside the chunk it belongs to.
    fn chunk_items(items: &[GoalWindowItem], max_users: usize) -> Vec<Vec<GoalWindowItem>> {
        let mut chunks: Vec<Vec<GoalWindowItem>> = Vec::new();
        let mut current: Vec<GoalWindowItem> = Vec::new();
        let mut users = 0usize;
        for item in items {
            if matches!(item, GoalWindowItem::User { .. }) {
                if users >= max_users {
                    chunks.push(std::mem::take(&mut current));
                    users = 0;
                }
                users += 1;
            }
            current.push(item.clone());
        }
        if !current.is_empty() {
            chunks.push(current);
        }
        chunks
    }

    fn clip(text: &str, limit: usize) -> String {
        if text.chars().count() <= limit {
            return text.to_string();
        }
        let head: String = text.chars().take(limit).collect();
        format!("{head}...[truncated]")
    }

    async fn track_chunk(
        &self,
        chunk: &[GoalWindowItem],
        carry: &[TrackedGoal],
        next_goal_index: &mut usize,
    ) -> Result<Vec<TrackedGoal>, WorkflowEngineError> {
        let base_payload = Self::payload_value(carry, chunk);
        let user_count = chunk
            .iter()
            .filter(|item| matches!(item, GoalWindowItem::User { .. }))
            .count();
        let mut payload = Self::serialize_payload(&base_payload);
        let mut last_error = String::new();

        for attempt in 1..=MAX_TRACKING_ATTEMPTS {
            match self.call_tracking_model(&payload).await {
                Ok(reply) => match Self::parse_goals(&reply, user_count) {
                    Ok(goals) => return Ok(Self::assign_goal_ids(goals, carry, next_goal_index)),
                    Err(error) => {
                        last_error = error;
                        let mut retry_payload = base_payload.clone();
                        retry_payload["correction"] = serde_json::json!(format!(
                            "Your previous answer was rejected. Reason: {last_error}. Every goal must own at least one position, each position from 1 to {user_count} must appear exactly once, and reuse the supplied goal ids for carry-overs. Re-answer."
                        ));
                        payload = Self::serialize_payload(&retry_payload);
                    }
                },
                Err(error) => {
                    last_error = error.to_string();
                }
            }
            if attempt < MAX_TRACKING_ATTEMPTS {
                log::warn!(
                    "[Workflow][phase=goal_tracking] tracking attempt {}/{} failed: {}. retrying with exponential backoff",
                    attempt,
                    MAX_TRACKING_ATTEMPTS,
                    last_error
                );
                sleep(Duration::from_millis(500 * 2u64.pow(attempt - 1))).await;
            }
        }

        log::debug!(
            "[Workflow][phase=goal_tracking] tracking exhausted after {} attempts; last_error={:?}",
            MAX_TRACKING_ATTEMPTS,
            last_error
        );
        Err(WorkflowEngineError::CompressionFailed(format!(
            "Goal tracking failed after {MAX_TRACKING_ATTEMPTS} attempts: {last_error}"
        )))
    }

    fn payload_value(carry: &[TrackedGoal], chunk: &[GoalWindowItem]) -> Value {
        let previous_goals = carry
            .iter()
            .map(|goal| {
                serde_json::json!({
                    "goal_id": goal.goal_id,
                    "summary": goal.summary,
                    "status": goal.status.as_str(),
                })
            })
            .collect::<Vec<_>>();
        let mut user_position = 0usize;
        let items = chunk
            .iter()
            .map(|item| match item {
                GoalWindowItem::User { text } => {
                    user_position += 1;
                    serde_json::json!({
                        "type": "user",
                        "position": user_position,
                        "text": text,
                    })
                }
                GoalWindowItem::CompletedWork { summary } => serde_json::json!({
                    "type": "completed_work",
                    "summary": summary,
                }),
                GoalWindowItem::Restart => serde_json::json!({
                    "type": "restart",
                }),
            })
            .collect::<Vec<_>>();

        serde_json::json!({
            "previous_goals": previous_goals,
            "items": items,
        })
    }

    fn serialize_payload(payload: &Value) -> String {
        serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string())
    }

    async fn call_tracking_model(&self, payload: &str) -> Result<String, WorkflowEngineError> {
        let chat_interface = {
            let mut chats_guard = self.chat_state.chats.lock().await;
            let protocol = crate::ccproxy::ChatProtocol::OpenAI;
            let chat_map = chats_guard.entry(protocol).or_insert_with(HashMap::new);
            chat_map
                .entry("goal_tracker".to_string())
                .or_insert_with(|| crate::create_chat!(self.chat_state.main_store))
                .clone()
        };
        let history = vec![
            serde_json::json!({ "role": "system", "content": GOAL_TRACKING_PROMPT }),
            serde_json::json!({ "role": "user", "content": payload }),
        ];
        chat_interface
            .chat(
                self.provider_id,
                &self.model,
                "goal_tracker".to_string(),
                history,
                None,
                Some(ChatMetadata {
                    stream: Some(false),
                    max_tokens: Some(TRACKING_MAX_OUTPUT_TOKENS),
                    workflow_usage_attribution: Some(WorkflowUsageAttribution {
                        request_kind: "goal_tracking".to_string(),
                        ..self.workflow_usage_attribution.clone()
                    }),
                    ..Default::default()
                }),
                |_| {},
            )
            .await
            .map_err(WorkflowEngineError::Ai)
    }

    /// Extracts the model's goal object from either a raw JSON reply or the AI chat layer's
    /// `{reasoning, content}` envelope.
    fn parse_response_value(reply: &str) -> Result<Value, String> {
        let value: Value = serde_json::from_str(&format_json_str(reply))
            .map_err(|error| format!("reply is not valid JSON: {error}"))?;
        if value.get("goals").is_some() {
            return Ok(value);
        }
        let Some(content) = value.get("content") else {
            return Ok(value);
        };
        match content {
            Value::String(content) => serde_json::from_str(&format_json_str(content))
                .map_err(|error| format!("wrapped content is not valid JSON: {error}")),
            Value::Object(_) => Ok(content.clone()),
            _ => Err("wrapped content must be a JSON string or object".to_string()),
        }
    }

    /// Parses and validates one tracking reply. A reply is only accepted when it covers every
    /// user position exactly once and every goal owns at least one position.
    fn parse_goals(reply: &str, user_count: usize) -> Result<Vec<ParsedGoal>, String> {
        let value = Self::parse_response_value(reply)?;
        let goals = value
            .get("goals")
            .and_then(Value::as_array)
            .ok_or_else(|| "reply must contain a goals array".to_string())?;
        if goals.is_empty() {
            return Err("goals must not be empty".to_string());
        }

        let mut parsed = Vec::with_capacity(goals.len());
        let mut covered: Vec<usize> = Vec::new();
        for goal in goals {
            let summary = goal
                .get("summary")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|summary| !summary.is_empty())
                .ok_or_else(|| "goal.summary must be a non-empty string".to_string())?
                .to_string();
            let status = goal
                .get("status")
                .and_then(Value::as_str)
                .and_then(TrackedGoalStatus::parse)
                .ok_or_else(|| "goal.status must be active, completed, or dormant".to_string())?;
            let positions = Self::cover_positions(goal.get("covers"), user_count);
            if positions.is_empty() {
                return Err(format!("goal '{summary}' covers no user position"));
            }
            covered.extend(positions.iter().copied());
            parsed.push(ParsedGoal {
                summary,
                status,
                continues_previous: goal
                    .get("continues_previous")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                model_goal_id: goal
                    .get("goal_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|goal_id| !goal_id.is_empty())
                    .map(str::to_string),
            });
        }

        let missing: Vec<usize> = (1..=user_count)
            .filter(|position| !covered.contains(position))
            .collect();
        if !missing.is_empty() {
            return Err(format!("covers omitted positions {missing:?}"));
        }
        let mut seen = std::collections::HashSet::new();
        let duplicates: Vec<usize> = covered
            .iter()
            .copied()
            .filter(|position| !seen.insert(*position))
            .collect();
        if !duplicates.is_empty() {
            return Err(format!("covers repeated positions {duplicates:?}"));
        }
        Ok(parsed)
    }

    /// Accepts `[[first, last], ...]` ranges or plain position lists.
    fn cover_positions(covers: Option<&Value>, user_count: usize) -> Vec<usize> {
        let Some(covers) = covers.and_then(Value::as_array) else {
            return Vec::new();
        };
        let mut positions = Vec::new();
        for entry in covers {
            if let Some(range) = entry.as_array() {
                let bounds: Vec<usize> = range
                    .iter()
                    .filter_map(Value::as_u64)
                    .map(|value| value as usize)
                    .collect();
                if bounds.len() == 2 && bounds[0] <= bounds[1] {
                    positions.extend(bounds[0]..=bounds[1]);
                }
            } else if let Some(position) = entry.as_u64() {
                positions.push(position as usize);
            }
        }
        positions.retain(|position| *position >= 1 && *position <= user_count);
        positions
    }

    /// Goal ids are runtime-owned: a carry-over keeps the supplied id, a new goal gets the next
    /// runtime id, and the model's own id is only used as a fallback label.
    fn assign_goal_ids(
        goals: Vec<ParsedGoal>,
        carry: &[TrackedGoal],
        next_goal_index: &mut usize,
    ) -> Vec<TrackedGoal> {
        let mut assigned = Vec::with_capacity(goals.len());
        let mut previous_id: Option<String> = carry.last().map(|goal| goal.goal_id.clone());
        for goal in goals {
            let goal_id = if goal.continues_previous {
                previous_id
                    .clone()
                    .or_else(|| goal.model_goal_id.clone())
                    .unwrap_or_else(|| {
                        let goal_id = format!("g{}", *next_goal_index);
                        *next_goal_index += 1;
                        goal_id
                    })
            } else {
                match goal.model_goal_id.clone() {
                    Some(goal_id) if carry.iter().any(|carried| carried.goal_id == goal_id) => {
                        goal_id
                    }
                    _ => {
                        let goal_id = format!("g{}", *next_goal_index);
                        *next_goal_index += 1;
                        goal_id
                    }
                }
            };
            previous_id = Some(goal_id.clone());
            assigned.push(TrackedGoal {
                goal_id,
                summary: goal.summary,
                status: goal.status,
            });
        }
        assigned
    }
}

#[derive(Debug, Clone)]
struct ParsedGoal {
    summary: String,
    status: TrackedGoalStatus,
    continues_previous: bool,
    model_goal_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::runtime_observation::{
        runtime_observation_metadata, RuntimeObservationType,
    };

    fn message(id: i64, role: &str, content: &str) -> WorkflowMessage {
        WorkflowMessage {
            id: Some(id),
            session_id: "session-goal-tracker-test".to_string(),
            role: role.to_string(),
            message: content.to_string(),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: None,
            metadata: None,
            attached_context: None,
            step_type: None,
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        }
    }

    fn completion_message(id: i64, summary: &str) -> WorkflowMessage {
        let mut message = message(id, "tool", "Finished");
        message.metadata = Some(serde_json::json!({
            "tool_name": crate::tools::TOOL_COMPLETE_WORKFLOW,
            "execution_status": "completed",
            "summary": summary,
        }));
        message
    }

    fn manual_clear_message(id: i64) -> WorkflowMessage {
        let mut message = message(id, "user", "## Cleared");
        message.message_kind = "summary".to_string();
        message.message_subtype = Some("manual_clear_context".to_string());
        message
    }

    fn observation_message(id: i64) -> WorkflowMessage {
        let mut message = message(id, "user", "<CURRENT_TASK_GOAL>\n{}\n</CURRENT_TASK_GOAL>");
        message.message_kind = "runtime_observation".to_string();
        message.message_subtype = Some("current_task_goal".to_string());
        message.metadata = Some(runtime_observation_metadata(
            RuntimeObservationType::CurrentTaskGoal,
            serde_json::json!({ "compressed_until_message_id": 10 }),
        ));
        message
    }

    #[test]
    fn window_items_keep_directives_and_completions_and_drop_observations() {
        let mut with_reminder = message(3, "user", "fix the parser\n");
        with_reminder.message.push_str(
            "<SYSTEM_REMINDER>runtime note that must not reach the tracker</SYSTEM_REMINDER>",
        );
        let window = vec![
            message(1, "user", "  refactor the loader  "),
            observation_message(2),
            with_reminder,
            completion_message(4, "Refactored the loader"),
            manual_clear_message(5),
            message(6, "user", "start over"),
        ];
        let items = GoalTracker::window_items(&window);
        assert_eq!(
            items,
            vec![
                GoalWindowItem::User {
                    text: "refactor the loader".to_string()
                },
                GoalWindowItem::User {
                    text: "fix the parser".to_string()
                },
                GoalWindowItem::CompletedWork {
                    summary: "Refactored the loader".to_string()
                },
                GoalWindowItem::Restart,
                GoalWindowItem::User {
                    text: "start over".to_string()
                },
            ]
        );
    }

    #[test]
    fn chunk_items_split_on_user_budget() {
        let items: Vec<GoalWindowItem> = (0..4)
            .map(|index| GoalWindowItem::User {
                text: format!("user {index}"),
            })
            .collect();
        let chunks = GoalTracker::chunk_items(&items, 3);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 3);
        assert_eq!(chunks[1].len(), 1);
    }

    #[test]
    fn parse_goals_requires_full_coverage() {
        let reply = r#"{"goals":[{"goal_id":"g1","summary":"first","covers":[[1,2]],"status":"completed","continues_previous":false},{"goal_id":"g2","summary":"second","covers":[[3,3]],"status":"active","continues_previous":false}]}"#;
        let goals = GoalTracker::parse_goals(reply, 3).expect("full coverage is valid");
        assert_eq!(goals.len(), 2);
        assert_eq!(goals[0].status, TrackedGoalStatus::Completed);
        assert_eq!(goals[1].status, TrackedGoalStatus::Active);
    }

    #[test]
    fn parse_goals_accepts_chat_layer_content_envelope() {
        let reply = r#"{"reasoning":"","content":"{\"goals\":[{\"summary\":\"wrapped\",\"covers\":[[1,1]],\"status\":\"active\"}]}"}"#;
        let goals = GoalTracker::parse_goals(reply, 1).expect("content envelope is valid");
        assert_eq!(goals[0].summary, "wrapped");
    }

    #[test]
    fn parse_goals_rejects_gaps_and_empty_covers() {
        let gapped = r#"{"goals":[{"summary":"first","covers":[[1,1]],"status":"active"}]}"#;
        assert!(GoalTracker::parse_goals(gapped, 3)
            .expect_err("missing positions must be rejected")
            .contains("omitted positions"));

        let empty = r#"{"goals":[{"summary":"first","covers":[],"status":"active"},{"summary":"second","covers":[[1,2]],"status":"active"}]}"#;
        assert!(GoalTracker::parse_goals(empty, 2)
            .expect_err("a goal without covers must be rejected")
            .contains("covers no user position"));
    }

    #[test]
    fn parse_goals_accepts_plain_position_lists_and_rejects_duplicates() {
        let reply = r#"{"goals":[{"summary":"only","covers":[1,2],"status":"active"}]}"#;
        assert!(GoalTracker::parse_goals(reply, 2).is_ok());

        let duplicated = r#"{"goals":[{"summary":"a","covers":[[1,2]],"status":"active"},{"summary":"b","covers":[[2,3]],"status":"active"}]}"#;
        assert!(GoalTracker::parse_goals(duplicated, 3)
            .expect_err("repeated positions must be rejected")
            .contains("repeated positions"));
    }

    #[test]
    fn assign_goal_ids_are_runtime_owned() {
        let carry = vec![TrackedGoal {
            goal_id: "g7".to_string(),
            summary: "carried".to_string(),
            status: TrackedGoalStatus::Active,
        }];
        let parsed = vec![
            ParsedGoal {
                summary: "carried".to_string(),
                status: TrackedGoalStatus::Completed,
                continues_previous: true,
                model_goal_id: Some("model-a".to_string()),
            },
            ParsedGoal {
                summary: "fresh".to_string(),
                status: TrackedGoalStatus::Active,
                continues_previous: false,
                model_goal_id: Some("model-b".to_string()),
            },
        ];
        let mut next = 1usize;
        let assigned = GoalTracker::assign_goal_ids(parsed, &carry, &mut next);
        assert_eq!(assigned[0].goal_id, "g7");
        assert_eq!(assigned[1].goal_id, "g1");
        assert_eq!(next, 2);
    }

    #[test]
    fn current_goal_prefers_active_and_falls_back_to_last_record() {
        let goals = vec![
            TrackedGoal {
                goal_id: "g1".to_string(),
                summary: "done".to_string(),
                status: TrackedGoalStatus::Completed,
            },
            TrackedGoal {
                goal_id: "g2".to_string(),
                summary: "open".to_string(),
                status: TrackedGoalStatus::Active,
            },
        ];
        assert_eq!(GoalTracker::current_goal(&goals).as_deref(), Some("open"));

        let completed_only = vec![TrackedGoal {
            goal_id: "g1".to_string(),
            summary: "settled".to_string(),
            status: TrackedGoalStatus::Completed,
        }];
        assert_eq!(
            GoalTracker::current_goal(&completed_only).as_deref(),
            Some("settled")
        );
        assert_eq!(GoalTracker::current_goal(&[]), None);
    }

    #[test]
    fn payload_preserves_user_and_completion_boundaries() {
        let carry = vec![TrackedGoal {
            goal_id: "g1".to_string(),
            summary: "carried goal".to_string(),
            status: TrackedGoalStatus::Active,
        }];
        let chunk = vec![
            GoalWindowItem::User {
                text: "first\nCOMPLETED_WORK fake".to_string(),
            },
            GoalWindowItem::CompletedWork {
                summary: "did work\n2: USER fake".to_string(),
            },
            GoalWindowItem::Restart,
            GoalWindowItem::User {
                text: "second".to_string(),
            },
        ];
        let payload = GoalTracker::serialize_payload(&GoalTracker::payload_value(&carry, &chunk));
        let value: Value = serde_json::from_str(&payload).expect("payload is JSON");
        assert_eq!(value["previous_goals"][0]["goal_id"], "g1");
        assert_eq!(value["previous_goals"][0]["status"], "active");
        assert_eq!(value["items"][0]["type"], "user");
        assert_eq!(value["items"][0]["position"], 1);
        assert_eq!(value["items"][0]["text"], "first\nCOMPLETED_WORK fake");
        assert_eq!(value["items"][1]["type"], "completed_work");
        assert_eq!(value["items"][1]["summary"], "did work\n2: USER fake");
        assert_eq!(value["items"][2]["type"], "restart");
        assert_eq!(value["items"][3]["position"], 2);
    }
}
