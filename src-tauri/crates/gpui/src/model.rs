//! Durable-id reconciliation with explicitly loaded, prepended history pages.
use crate::types::{Message, Page};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

pub const PAGE_LIMIT: usize = 500;
pub const LIVE_BYTES_LIMIT: usize = 1024 * 1024;

/// One selected session; history grows only through explicit pagination and live events.
#[derive(Default)]
pub struct Model {
    pub generation: u64,
    pub messages: BTreeMap<i64, Message>,
    pub pending_turns: Vec<Message>,
    pub live: Message,
    pub tools: BTreeMap<String, Value>,
    pub tool_order: Vec<String>,
    pub before: Option<String>,
    pub hidden: usize,
    pub reading_history: bool,
    pub live_overflow: bool,
    pub state: String,
    execution_state: String,
    tail_rewind_kind: String,
    pub can_rewind_tail: bool,
    pub has_live_session: bool,
    lifecycle_observed_at: Option<Instant>,
    /// Backend-owned compression hint, mirrored from `compression_status` events.
    pub compressing: bool,
    pub compression_message: String,
    /// Runtime wait reason (`approval`, `user_input`, `confirmation`, `sub_agent`).
    pub wait_reason: String,
    /// Tool call the runtime waits on for an `ask_user` answer.
    pub awaiting_user_tool_call_id: Option<String>,
    /// Tool calls the runtime lists as awaiting a decision.
    pub pending_tool_ids: BTreeSet<String>,
    /// When each live tool call was first seen running, for the elapsed label.
    pub tool_started: BTreeMap<String, Instant>,
    /// When each live tool call finished, so a completion stays visible briefly.
    pub tool_finished: BTreeMap<String, Instant>,
    /// Authorized roots of the selected workflow, used to shorten tool paths
    /// and to fill the authorized-paths tab.
    pub roots: Vec<String>,
    /// Last workflow payload, which carries `agent_config` for the composer's
    /// agent, model and sandbox selectors.
    pub workflow: Value,
    pub todo_list: Vec<Value>,
    pub sub_agent_progress: BTreeMap<String, Value>,
    pub current_context_tokens: Option<u64>,
    pub max_context_tokens: Option<u64>,
    /// A live `context_usage` event owns the token figures. Periodic snapshot
    /// reconciliation must not replace them with the persisted, lagging copy.
    context_usage_from_event: bool,
    pub queued_messages: Vec<QueuedMessage>,
    /// Queue rows whose removal signal is in flight to the runtime.
    removing_queue_ids: BTreeMap<String, (Instant, bool)>,
    queue_removal_results: BTreeMap<String, bool>,
    finished_queue_ids: BTreeSet<String>,
    snapshot_queue_ids: BTreeSet<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct QueuedMessage {
    #[serde(rename = "queued_user_message_id")]
    pub id: String,
    pub content: String,
    #[serde(default)]
    pub metadata: Value,
    #[serde(skip)]
    pub provisional: bool,
}

fn finished_tool_id(row: &Message) -> Option<&str> {
    let metadata = &row.metadata;
    let status = metadata["execution_status"].as_str();
    let finished = matches!(status, Some("completed" | "failed" | "rejected"))
        || matches!(metadata["type"].as_str(), Some("tool_completed" | "tool_failed"))
        || (row.role == "tool" && status.is_none() && metadata["approval_status"] != "pending");
    finished.then(|| metadata["tool_call_id"].as_str()).flatten()
}

impl Model {
    pub fn select(&mut self) -> u64 {
        let generation = self.generation.wrapping_add(1);
        *self = Self {
            generation,
            ..Self::default()
        };
        generation
    }

    pub fn apply_context_clear(&mut self, response: &Value) -> bool {
        let Ok(marker) = serde_json::from_value::<Message>(response["markerMessage"].clone()) else { return false; };
        if marker.id <= 0 || marker.role != "system" || marker.message_kind != "summary"
            || marker.message_subtype.as_deref() != Some("manual_clear_context") { return false; }
        self.messages.insert(marker.id, marker);
        self.state = response["state"].as_str().unwrap_or("pending").to_owned();
        self.execution_state = self.state.clone();
        self.tail_rewind_kind.clear();
        self.has_live_session = response["hasLiveSession"].as_bool().unwrap_or(false);
        self.lifecycle_observed_at = Some(Instant::now());
        self.wait_reason.clear();
        self.pending_tool_ids.clear();
        self.awaiting_user_tool_call_id = None;
        self.live = Message::default();
        self.live_overflow = false;
        true
    }

    pub fn can_clear_context(&self) -> bool {
        self.accepts_idle_turn()
    }

    pub fn accepts_idle_turn(&self) -> bool {
        matches!(self.state.as_str(), "completed" | "error" | "failed" | "cancelled")
            || (self.state == "pending"
                && !self.has_live_session
                && (self.execution_state.is_empty() || self.execution_state == "pending")
                && !matches!(self.wait_reason.as_str(), "approval" | "user_input" | "confirmation" | "sub_agent"))
    }

    pub fn turn_submission_pending(&self) -> bool {
        self.pending_turns.iter().any(|row| row.metadata["gpui_turn_status"] == "pending")
            || self.queued_messages.iter().any(|item| item.provisional)
    }

    pub fn can_continue(&self) -> bool {
        !self.has_live_session
            && !self.turn_submission_pending()
            && !matches!(self.tail_rewind_kind.as_str(), "ask_user_wait" | "ask_user_answered")
            && matches!(self.state.as_str(), "thinking" | "executing" | "auditing" | "running" | "paused" | "awaiting_sub_agent" | "error" | "failed" | "cancelled")
    }

    pub fn can_stop(&self) -> bool {
        self.has_live_session
            && matches!(self.state.as_str(), "thinking" | "executing" | "auditing" | "running"
                | "paused" | "awaiting_user" | "awaiting_approval" | "awaiting_auto_approval" | "awaiting_sub_agent")
    }

    pub fn accepts(&self, generation: u64) -> bool {
        generation == self.generation
    }

    pub fn page(&mut self, mut page: Page, older: bool) {
        if older {
            self.before = page.message_window_before_id;
            self.hidden = page.hidden_earlier_message_count;
            self.messages
                .extend(page.messages.into_iter().map(|m| (m.id, m)));
            self.reading_history = true;
            return;
        }
        if let Some(status) = page.workflow["status"].as_str() {
            page.status = status.to_owned();
        }
        // A snapshot begun before a live state event can still reconcile history,
        // but it must not replace the newer lifecycle state used by the footer.
        let stale_lifecycle = page.fetched_from.zip(self.lifecycle_observed_at)
            .is_some_and(|(started, observed)| started <= observed);
        if !stale_lifecycle && !page.status.is_empty() {
            self.state = page.status.clone();
        }
        if !page.workflow.is_null() {
            self.workflow = page.workflow.clone();
            let todo = page.workflow.get("todo_list").or_else(|| page.workflow.get("todoList"));
            self.todo_list = todo.and_then(|value| {
                value.as_array().cloned().or_else(|| serde_json::from_str(value.as_str()?).ok())
            }).unwrap_or_default();
        }
        // Older-message pages may omit the workflow payload; keep the roots the
        // snapshot established instead of clearing them on every page.
        if page.workflow.get("agent_config").is_some() || page.workflow.get("agentConfig").is_some()
        {
            self.roots = crate::types::workflow_roots(&page.workflow);
        }
        let context = &page.execution_context;
        let snapshot_current = context["current_context_tokens"]
            .as_u64()
            .or_else(|| context["currentContextTokens"].as_u64());
        let snapshot_max = context["max_context_tokens"]
            .as_u64()
            .or_else(|| context["maxContextTokens"].as_u64());
        // Snapshots are fetched every few seconds to reconcile messages. Their
        // execution context is persisted at checkpoints, so it can be much
        // older than the live `context_usage` stream; letting it win made the
        // panel alternate between two percentages.
        if self.context_usage_from_event {
            self.current_context_tokens = self.current_context_tokens.or(snapshot_current);
            self.max_context_tokens = self.max_context_tokens.or(snapshot_max);
        } else {
            self.current_context_tokens = snapshot_current;
            self.max_context_tokens = snapshot_max;
        }
        if !stale_lifecycle {
            self.execution_state = context["state"].as_str().unwrap_or("pending").to_ascii_lowercase();
            self.tail_rewind_kind = page.tail_rewind_kind.unwrap_or_default();
            self.can_rewind_tail = page.can_rewind_tail;
            self.has_live_session = page.has_live_session;
            self.wait_reason = context["waitReason"]
                .as_str()
                .or_else(|| context["wait_reason"].as_str())
                .unwrap_or("")
                .to_owned();
            self.awaiting_user_tool_call_id = context["awaitingUserToolCallId"]
                .as_str()
                .or_else(|| context["awaiting_user_tool_call_id"].as_str())
                .filter(|id| !id.is_empty())
                .map(str::to_owned);
        }
        self.reconcile_queue(context, &page.messages);
        self.pending_tool_ids = context["pendingTools"]
            .as_array()
            .or_else(|| context["pending_tools"].as_array())
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|tool| {
                        tool["toolCallId"]
                            .as_str()
                            .or_else(|| tool["tool_call_id"].as_str())
                    })
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let initial = self.messages.is_empty();
        let finalized_live = page
            .messages
            .iter()
            .any(|row| row.role == "assistant" && !self.messages.contains_key(&row.id));
        if finalized_live
            || matches!(
                page.status.as_str(),
                "completed" | "failed" | "cancelled" | "error"
            )
        {
            self.live = Message::default();
            self.live_overflow = false;
        }
        if initial {
            self.before = page.message_window_before_id;
            self.hidden = page.hidden_earlier_message_count;
        }
        for row in &page.messages {
            if row.role == "user" {
                if let Some(id) = row.metadata["client_message_id"].as_str() {
                    self.confirm_turn(id);
                }
            }
        }
        // Snapshot windows are partial: never replace already loaded history.
        self.messages
            .extend(page.messages.into_iter().map(|m| (m.id, m)));
        if initial {
            self.live = Message::default();
        }
        for row in self.messages.values() {
            if let Some(id) = finished_tool_id(row) {
                self.tools.remove(id);
                self.tool_started.remove(id);
            }
        }
        self.tool_order.retain(|id| self.tools.contains_key(id));
    }

    /// A transport failure can race with a runtime acknowledgement.
    pub fn reject_queue_submission(&mut self, id: &str) -> bool {
        let before = self.queued_messages.len();
        self.queued_messages
            .retain(|item| item.id != id || !item.provisional);
        if !self.queued_messages.iter().any(|item| item.id == id) {
            self.removing_queue_ids.remove(id);
        }
        self.queued_messages.len() != before
    }

    /// A local removal is not a cancellation receipt, even if the row disappears.
    pub fn can_remove_queued_message(&self, id: &str) -> bool {
        !self.queue_removal_pending(id)
            && self.queued_messages.iter().any(|item| item.id == id && !item.provisional)
    }

    pub fn mark_queue_removal(&mut self, id: &str, requested: Instant, edit: bool) {
        self.removing_queue_ids.insert(id.to_owned(), (requested, edit));
    }

    pub fn queue_removal_pending(&self, id: &str) -> bool {
        self.removing_queue_ids.contains_key(id)
    }

    pub fn reject_queue_removal(&mut self, id: &str, requested: Instant) -> bool {
        self.removing_queue_ids.get(id).is_some_and(|(at, _)| *at == requested)
            && self.removing_queue_ids.remove(id).is_some()
    }

    pub fn expire_queue_removals(&mut self, now: Instant) -> bool {
        let before = self.removing_queue_ids.len();
        self.removing_queue_ids.retain(|_, (requested, _)| {
            now.saturating_duration_since(*requested) < std::time::Duration::from_secs(30)
        });
        self.removing_queue_ids.len() != before
    }

    pub fn take_queue_removal_result(&mut self, id: &str) -> Option<bool> {
        self.queue_removal_results.remove(id)
    }

    fn finish_queued_message(&mut self, id: &str) {
        self.finished_queue_ids.insert(id.to_owned());
        self.removing_queue_ids.remove(id);
        self.queued_messages.retain(|item| item.id != id);
    }

    fn apply_queued_message(&mut self, id: &str) {
        if self.removing_queue_ids.get(id).is_some_and(|(_, edit)| *edit)
            || self.queue_removal_results.contains_key(id)
        {
            self.queue_removal_results.insert(id.to_owned(), false);
        }
        self.finish_queued_message(id);
    }

    fn cancel_queued_message(&mut self, id: &str) {
        if self.removing_queue_ids.get(id).is_some_and(|(_, edit)| *edit) {
            self.queue_removal_results.entry(id.to_owned()).or_insert(true);
        }
        self.finish_queued_message(id);
    }

    fn reconcile_queue(&mut self, context: &Value, messages: &[Message]) {
        for id in messages.iter().filter(|row| row.role == "user").flat_map(|row| {
            ["queued_user_message_id", "client_message_id"]
                .into_iter()
                .filter_map(|key| row.metadata[key].as_str())
        }) {
            self.apply_queued_message(id);
        }
        for id in context["removed_queued_user_message_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            // Legacy tombstones prevent revival, but can coexist with an already
            // applied turn. Only the explicit result can reopen an edit draft.
            self.finished_queue_ids.insert(id.to_owned());
            if self.removing_queue_ids.get(id).is_some_and(|(_, edit)| !edit) {
                self.removing_queue_ids.remove(id);
            }
        }
        let items = if context.is_null() {
            Vec::new()
        } else if let Some(items) = context["queued_user_messages"].as_array() {
            items
                .iter()
                .filter_map(|item| serde_json::from_value::<QueuedMessage>(item.clone()).ok())
                .collect()
        } else {
            Vec::new()
        };
        let mut ids = BTreeSet::new();
        let mut queue: Vec<_> = items
            .into_iter()
            .filter(|item| {
                !item.id.is_empty()
                    && !self.finished_queue_ids.contains(&item.id)
                    && ids.insert(item.id.clone())
            })
            .collect();
        // An accepted input can be visible on SSE before it reaches the persisted queue.
        queue.extend(
            std::mem::take(&mut self.queued_messages)
                .into_iter()
                .filter(|item| {
                    !ids.contains(&item.id)
                        && !self.snapshot_queue_ids.contains(&item.id)
                        && !self.finished_queue_ids.contains(&item.id)
                        && (item.provisional || !crate::chimes::terminal(&self.state))
                }),
        );
        self.snapshot_queue_ids = ids;
        self.queued_messages = queue;
    }

    /// Drops the local echo only when the backend supplies a durable user row.
    fn confirm_turn(&mut self, id: &str) {
        self.pending_turns.retain(|row| row.metadata["client_message_id"].as_str() != Some(id));
    }

    pub fn mark_turn_unconfirmed(&mut self, id: &str) -> bool {
        let Some(row) = self.pending_turns.iter_mut().find(|row| row.metadata["client_message_id"].as_str() == Some(id)) else {
            return false;
        };
        row.metadata["gpui_turn_status"] = json!("unconfirmed");
        true
    }

    pub fn resume_follow(&mut self) {
        self.reading_history = false;
    }

    pub fn visible(&self) -> impl Iterator<Item = &Message> {
        self.messages.values()
    }

    /// Negative/zero deltas include downward and inert notifications, not reading intent.
    pub fn pause_for_scroll(&mut self, delta_y: f32) -> bool {
        if delta_y <= 0.0 {
            return false;
        }
        self.freeze();
        true
    }

    pub fn freeze(&mut self) {
        self.reading_history = true;
    }

    /// Seed only calls the runtime has started but not yet finished. A missing
    /// audit timestamp leaves the tool visible without inventing a start time.
    pub fn restore_running_tools(&mut self, tools: Vec<(Value, Option<Instant>)>) {
        if !self.has_live_session || crate::chimes::terminal(&self.state) {
            return;
        }
        for (tool, started) in tools {
            let Some(id) = tool["tool_call_id"].as_str().map(str::to_owned) else {
                continue;
            };
            if self.tool_finished.contains_key(&id)
                || self.tools.get(&id).is_some_and(|tool| {
                    matches!(tool["type"].as_str(), Some("tool_started" | "tool_completed" | "tool_failed"))
                })
                || self.messages.values().any(|row| finished_tool_id(row) == Some(id.as_str()))
            {
                continue;
            }
            self.event(tool);
            if let Some(started) = started {
                self.tool_started.insert(id, started);
            } else {
                self.tool_started.remove(&id);
            }
        }
    }

    pub fn clear_compression_hint(&mut self) {
        self.compressing = false;
        self.compression_message.clear();
    }

    pub fn reset_live(&mut self) {
        self.clear_compression_hint();
        self.live = Message::default();
        self.tools.clear();
        self.tool_order.clear();
        self.tool_started.clear();
        self.tool_finished.clear();
        self.live_overflow = false;
        // The stream restart is followed by a fresh snapshot; let it seed the
        // token figures again until the next live usage event arrives.
        self.context_usage_from_event = false;
    }

    fn delta(&mut self, text: &str, reasoning: bool) {
        // Stop caching this transient output at the cap; durable finals remain intact.
        // The explicit overflow indicator tells readers to await reconciliation.
        let bytes = self.live.message.len() + self.live.reasoning.as_ref().map_or(0, String::len);
        if self.live_overflow || bytes.saturating_add(text.len()) > LIVE_BYTES_LIMIT {
            self.live_overflow = true;
            return;
        }
        if reasoning {
            self.live
                .reasoning
                .get_or_insert_with(String::new)
                .push_str(text);
        } else {
            self.live.message.push_str(text);
        }
    }

    pub fn event(&mut self, event: Value) {
        let kind = event["type"].as_str().unwrap_or_default();
        match kind {
            "sub_agent_progress" => {
                if let Some(id) = event["sub_agent_id"].as_str().filter(|id| !id.is_empty()) {
                    let progress = self.sub_agent_progress.entry(id.to_owned()).or_insert_with(|| serde_json::json!({}));
                    let incoming = event["updated_at_ms"].as_i64();
                    if incoming.zip(progress["updated_at_ms"].as_i64()).is_some_and(|(new, old)| new < old) {
                        return;
                    }
                    if let Some(timestamp) = incoming {
                        progress["updated_at_ms"] = timestamp.into();
                    }
                    for key in ["agent_name", "task", "title", "workflow_state", "status", "summary", "wait_reason"] {
                        if event[key].as_str().is_some_and(|value| !value.is_empty()) {
                            progress[key] = event[key].clone();
                        }
                    }
                    for key in ["tool_calls_count", "current_context_tokens", "max_context_tokens"] {
                        if event[key].as_u64().is_some() {
                            progress[key] = event[key].clone();
                        }
                    }
                }
            }
            "sync_todo" => {
                if let Some(items) = event["todo_list"].as_array() {
                    self.todo_list.clone_from(items);
                }
            }
            "context_usage" => {
                if let Some(tokens) = event["current_context_tokens"]
                    .as_u64()
                    .or_else(|| event["total_tokens"].as_u64())
                {
                    self.current_context_tokens = Some(tokens);
                    self.context_usage_from_event = true;
                }
                if let Some(tokens) = event["max_context_tokens"].as_u64() {
                    self.max_context_tokens = Some(tokens);
                    self.context_usage_from_event = true;
                }
            }
            "chunk" | "reasoning_chunk" => self.delta(
                event["content"].as_str().unwrap_or_default(),
                kind == "reasoning_chunk",
            ),
            "queued_user_message_removed" => {
                if let Some(id) = event["queued_user_message_id"].as_str() {
                    match event["cancelled_before_apply"].as_bool() {
                        Some(true) => self.cancel_queued_message(id),
                        Some(false) => self.apply_queued_message(id),
                        None => {
                            self.finished_queue_ids.insert(id.to_owned());
                            self.queued_messages.retain(|item| item.id != id);
                        }
                    }
                }
            }
            "message" => {
                if event["role"] == "user" {
                    if let Some(id) = event["metadata"]["queued_user_message_id"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                    {
                        if event["metadata"]["queue_status"] == "queued" {
                            if !self.finished_queue_ids.contains(id) {
                                let confirmed = QueuedMessage {
                                    id: id.to_owned(),
                                    content: event["content"]
                                        .as_str()
                                        .unwrap_or_default()
                                        .to_owned(),
                                    metadata: event["metadata"].clone(),
                                    provisional: false,
                                };
                                if let Some(item) =
                                    self.queued_messages.iter_mut().find(|item| item.id == id)
                                {
                                    *item = confirmed;
                                } else {
                                    self.queued_messages.push(confirmed);
                                }
                            }
                            return;
                        }
                        if event["metadata"]["queue_status"] == "applied" {
                            self.apply_queued_message(id);
                        }
                    }
                }
                if let Some(id) = event["message_id"]
                    .as_str()
                    .and_then(|s| s.parse::<i64>().ok())
                {
                    if event["role"] == "user" {
                        if let Some(client_id) = event["metadata"]["client_message_id"].as_str() {
                            self.confirm_turn(client_id);
                            self.apply_queued_message(client_id);
                        }
                    }
                    let row = Message {
                        id,
                        role: event["role"].as_str().unwrap_or_default().to_owned(),
                        message: event["content"].as_str().unwrap_or_default().to_owned(),
                        reasoning: event["reasoning"].as_str().map(str::to_owned),
                        metadata: event["metadata"].clone(),
                        ..Message::default()
                    };
                    if let Some(tool) = finished_tool_id(&row) {
                        self.tools.remove(tool);
                        self.tool_started.remove(tool);
                    }
                    if row.role == "assistant" {
                        self.live = Message::default();
                        self.live_overflow = false;
                    }
                    // A partial SSE message must not replace the typed clear marker.
                    if !self.messages.get(&id).is_some_and(|existing| {
                        existing.message_subtype.as_deref() == Some("manual_clear_context")
                    }) {
                        self.messages.insert(id, row);
                    }
                }
            }
            "confirm" | "approval_requested" => {
                if let Some(id) = event["tool_call_id"]
                    .as_str()
                    .or_else(|| event["id"].as_str())
                {
                    let mut preview = event.clone();
                    preview["tool_call_id"] = json!(id);
                    preview["execution_status"] = json!("pending_approval");
                    self.pending_tool_ids.insert(id.to_owned());
                    if !self.tools.contains_key(id) {
                        self.tool_order.push(id.to_owned());
                    }
                    self.tools.insert(id.to_owned(), preview);
                }
            }
            "approval_resolved" => {
                if let Some(id) = event["tool_call_id"].as_str() {
                    self.pending_tool_ids.remove(id);
                    self.tools.remove(id);
                    self.tool_order.retain(|tool| tool != id);
                    for row in self.messages.values_mut().rev() {
                        if row.metadata["tool_call_id"].as_str() == Some(id) {
                            row.metadata["approval_status"] = event["approval_status"].clone();
                            row.metadata["execution_status"] =
                                json!(if event["approved"].as_bool() == Some(true) {
                                    "approval_submitted"
                                } else {
                                    "rejected"
                                });
                            break;
                        }
                    }
                }
            }
            "tool_started" | "tool_completed" | "tool_failed" => {
                if let Some(id) = event["tool_call_id"].as_str() {
                    self.pending_tool_ids.remove(id);
                    self.tool_order.retain(|id| self.tools.contains_key(id));
                    if !self.tools.contains_key(id) {
                        self.tool_order.push(id.to_owned());
                    }
                    let card = self.tools.entry(id.to_owned()).or_insert_with(|| json!({}));
                    if let (Some(card), Some(fields)) = (card.as_object_mut(), event.as_object()) {
                        card.extend(
                            fields
                                .iter()
                                .map(|(key, value)| (key.clone(), value.clone())),
                        );
                    }
                    while self.tools.len() > 32 {
                        if !self.tool_order.is_empty() {
                            let oldest = self.tool_order.remove(0);
                            self.tools.remove(&oldest);
                        }
                    }
                    // The live status line reports the elapsed time of the tool
                    // it names, so the start and finish moments are tracked here.
                    if kind == "tool_started" {
                        self.tool_started
                            .entry(id.to_owned())
                            .or_insert_with(Instant::now);
                        self.tool_finished.remove(id);
                    } else {
                        self.tool_started.remove(id);
                        self.tool_finished.insert(id.to_owned(), Instant::now());
                    }
                    while self.tool_started.len() > 32 {
                        self.tool_started.pop_first();
                    }
                    while self.tool_finished.len() > 32 {
                        self.tool_finished.pop_first();
                    }
                }
            }
            "state" => {
                if let Some(state) = event["state"].as_str().filter(|state| !state.is_empty()) {
                    // The stream can report a restarted session before the next snapshot.
                    // Keep the footer from pairing its new state with the old liveness flag.
                    self.has_live_session = !crate::chimes::terminal(state);
                    self.state = state.to_owned();
                    self.lifecycle_observed_at = Some(Instant::now());
                }
                self.wait_reason = event["wait_reason"].as_str().unwrap_or_default().to_owned();
                let call_id = event.get("awaiting_user_tool_call_id");
                if self.wait_reason != "user_input" || call_id.is_some() {
                    self.awaiting_user_tool_call_id = call_id
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .filter(|_| self.wait_reason == "user_input")
                        .map(str::to_owned);
                }
            }
            "compression_status" => {
                self.compressing = event["is_compressing"].as_bool().unwrap_or(false);
                self.compression_message = event["message"].as_str().unwrap_or_default().to_owned();
            }
            "compression_applied" => {
                self.compressing = false;
                self.compression_message.clear();
                if let Some(tokens) = event["current_context_tokens"].as_u64() {
                    self.current_context_tokens = Some(tokens);
                    self.context_usage_from_event = true;
                }
                if let Some(tokens) = event["max_context_tokens"].as_u64() {
                    self.max_context_tokens = Some(tokens);
                    self.context_usage_from_event = true;
                }
            }
            _ => {}
        }
    }

    pub fn demo_history(session: &str, before: i64) -> Page {
        let mut page = Self::demo_page();
        let end = (before - 1).max(0);
        let start = (end - PAGE_LIMIT as i64 + 1).max(1);
        page.messages.retain(|row| row.id <= end - start + 1);
        for row in &mut page.messages {
            row.id += start - 1;
            row.message = format!("{session} · {}\n\n{}", row.id, row.message);
        }
        if start > 1 {
            page.message_window_before_id = Some(start.to_string());
            page.hidden_earlier_message_count = (start - 1) as usize;
        }
        page
    }

    pub fn demo_page() -> Page {
        Page {
            messages: (1..=PAGE_LIMIT as i64).map(|id| Message {
                id,
                role: "assistant".into(),
                message: format!("## Replay {id}\n\nA deterministic **Markdown** row.\n\n```rust\nlet answer = 42;\n```\n\n- first\n- second"),
                reasoning: Some("Bounded replay reasoning.".into()),
                metadata: json!({"tool_call_id": format!("demo-{id}"), "tool_name": "read_only_demo"}),
                ..Message::default()
            }).collect(),
            ..Page::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_progress_keeps_context_when_terminal_event_omits_it() {
        let mut model = Model::default();
        model.event(json!({"type":"sub_agent_progress","sub_agent_id":"child","tool_calls_count":2,"current_context_tokens":50,"max_context_tokens":200}));
        assert_eq!(model.sub_agent_progress["child"]["tool_calls_count"], 2);
        model.event(json!({"type":"sub_agent_progress","sub_agent_id":"child","tool_calls_count":3,"updated_at_ms":20}));
        model.event(json!({"type":"sub_agent_progress","sub_agent_id":"child","tool_calls_count":1,"updated_at_ms":10}));
        assert_eq!(model.sub_agent_progress["child"]["tool_calls_count"], 3);
        assert_eq!(model.sub_agent_progress["child"]["current_context_tokens"], 50);
        assert_eq!(model.sub_agent_progress["child"]["max_context_tokens"], 200);
        model.select();
        assert!(model.sub_agent_progress.is_empty());
    }

    #[test]
    fn status_details_hydrate_then_follow_runtime_events() {
        let mut model = Model::default();
        model.page(Page {
            workflow: json!({"todoList": "[{\"subject\":\"First\",\"status\":\"pending\"}]"}),
            execution_context: json!({"currentContextTokens": 10, "maxContextTokens": 100}),
            ..Page::default()
        }, false);
        assert_eq!(model.todo_list[0]["subject"], "First");
        assert_eq!(model.current_context_tokens, Some(10));
        model.event(json!({"type":"sync_todo","todo_list":[]}));
        model.event(json!({"type":"context_usage","current_context_tokens":25}));
        assert!(model.todo_list.is_empty());
        assert_eq!(model.current_context_tokens, Some(25));
        assert_eq!(model.max_context_tokens, Some(100));
    }

    #[test]
    fn live_context_usage_survives_periodic_snapshot_reconciliation() {
        let mut model = Model::default();
        let page = |current: u64| Page {
            execution_context: json!({
                "current_context_tokens": current,
                "max_context_tokens": 128000,
            }),
            ..Page::default()
        };
        model.page(page(1000), false);
        assert_eq!(model.current_context_tokens, Some(1000));
        model.event(json!({
            "type": "context_usage",
            "current_context_tokens": 48000,
            "max_context_tokens": 128000,
        }));
        // The five-second reconciliation snapshot is persisted at a checkpoint
        // and must not drag the live value back down.
        model.page(page(9000), false);
        assert_eq!(model.current_context_tokens, Some(48000));
        assert_eq!(model.max_context_tokens, Some(128000));
        // A reconnect drops the live claim so the next snapshot seeds it again.
        model.reset_live();
        model.page(page(7000), false);
        assert_eq!(model.current_context_tokens, Some(7000));
    }

    #[test]
    fn compression_applied_replaces_the_context_figures() {
        let mut model = Model::default();
        model.event(json!({
            "type": "context_usage",
            "current_context_tokens": 90000,
            "max_context_tokens": 128000,
        }));
        model.event(json!({
            "type": "compression_applied",
            "compressed_until_message_id": 12,
            "current_context_tokens": 12000,
            "max_context_tokens": 128000,
        }));
        assert_eq!(model.current_context_tokens, Some(12000));
        assert_eq!(model.max_context_tokens, Some(128000));
    }

    #[test]
    fn footer_actions_follow_snapshot_execution_and_live_state() {
        let mut model = Model::default();
        let page = |status: &str, context: Value, live, rewind: Option<&str>| Page {
            status: status.into(),
            execution_context: context,
            has_live_session: live,
            tail_rewind_kind: rewind.map(str::to_owned),
            ..Page::default()
        };
        model.page(page("pending", json!({"state": "executing"}), false, None), false);
        assert!(!model.can_clear_context());
        model.page(page("pending", json!({"state": "pending", "waitReason": "approval"}), false, None), false);
        assert!(!model.can_clear_context());
        model.page(page("pending", json!({"state": "pending"}), false, None), false);
        assert!(model.can_clear_context());
        for (status, reason) in [
            ("awaiting_approval", "approval"),
            ("awaiting_user", "user_input"),
            ("awaiting_sub_agent", "sub_agent"),
        ] {
            model.page(page(status, json!({"waitReason": reason}), true, None), false);
            assert!(model.can_stop(), "live {status}");
            model.page(page(status, json!({"waitReason": reason}), false, None), false);
            assert!(!model.can_stop(), "durable-only {status}");
        }
        model.page(page("paused", Value::Null, true, None), false);
        assert!(!model.can_continue());
        model.page(page("paused", Value::Null, false, Some("ask_user_wait")), false);
        assert!(!model.can_continue());
        model.page(page("paused", Value::Null, false, None), false);
        assert!(model.can_continue());
    }

    #[test]
    fn restarted_workflow_offers_stop_on_live_state_without_waiting_for_a_snapshot() {
        let mut model = Model::default();
        model.page(Page { status: "completed".into(), ..Page::default() }, false);
        assert!(!model.can_continue());
        assert!(!model.can_stop());

        for state in ["thinking", "executing", "awaiting_user"] {
            model.event(json!({"type": "state", "state": state}));
            assert!(!model.can_continue(), "{state}");
            assert!(model.can_stop(), "{state}");
        }

        model.event(json!({"type": "state", "state": "completed"}));
        assert!(!model.has_live_session);
        assert!(!model.can_stop());
        assert!(!model.can_continue());

        model.event(json!({"type": "state", "state": "error"}));
        assert!(!model.can_stop());
        assert!(model.can_continue());
    }

    #[test]
    fn continue_is_hidden_until_submit_settles_and_old_snapshot_cannot_hide_stop() {
        let mut model = Model::default();
        let old = || Page {
            fetched_from: Some(Instant::now() - std::time::Duration::from_secs(1)),
            status: "executing".into(),
            ..Page::default()
        };
        model.page(Page { status: "failed".into(), ..Page::default() }, false);
        assert!(model.can_continue());
        model.pending_turns.push(Message {
            metadata: json!({"gpui_turn_status":"pending", "client_message_id":"turn"}),
            ..Message::default()
        });
        model.page(old(), false);
        assert!(!model.can_continue());
        assert!(!model.can_stop());

        let stale = old();
        model.event(json!({"type":"state", "state":"thinking"}));
        model.event(json!({"type":"message", "role":"user", "message_id":"42",
            "metadata":{"client_message_id":"turn"}}));
        model.page(stale, false);
        assert!(!model.turn_submission_pending());
        assert!(!model.can_continue());
        assert!(model.can_stop());

        model.page(Page {
            fetched_from: model.lifecycle_observed_at.map(|at| at + std::time::Duration::from_nanos(1)),
            status: "cancelled".into(),
            ..Page::default()
        }, false);
        assert!(model.can_continue());
        assert!(!model.can_stop());
    }

    #[test]
    fn local_turn_reconciles_from_event_or_snapshot_without_text_deduplication() {
        for snapshot_first in [false, true] {
            let mut model = Model::default();
            let row = |id, client_id| Message {
                id,
                role: "user".into(),
                message: "same text".into(),
                metadata: json!({"client_message_id": client_id}),
                ..Message::default()
            };
            model.pending_turns.extend([row(0, "one"), row(0, "two")]);
            model.page(Page::default(), false);
            assert_eq!(model.pending_turns.len(), 2);
            assert!(model.mark_turn_unconfirmed("one"));
            let snapshot = || Page { messages: vec![row(42, "one")], ..Page::default() };
            let event = || json!({"type":"message", "message_id":"42", "role":"user", "content":"same text", "metadata":{"client_message_id":"one"}});
            if snapshot_first {
                model.page(snapshot(), false);
                model.event(event());
            } else {
                model.event(event());
                model.page(snapshot(), false);
            }
            assert_eq!(model.visible().count(), 1);
            assert!(!model.mark_turn_unconfirmed("one"));
            assert_eq!(model.pending_turns.len(), 1);
            assert_eq!(model.pending_turns[0].metadata["client_message_id"], "two");
            model.select();
            assert!(model.pending_turns.is_empty());
        }
    }

    #[test]
    fn clear_receipt_installs_one_durable_marker_before_snapshot() {
        let mut model = Model::default();
        model.messages.insert(1, Message { id: 1, ..Message::default() });
        let id = 9_007_199_254_740_993_i64;
        let receipt = json!({"state":"pending","hasLiveSession":false,"markerMessage": {
            "id":id,"role":"system","messageKind":"summary","messageSubtype":"manual_clear_context"
        }});
        assert!(!model.apply_context_clear(&json!({"markerMessage":null})));
        assert!(model.apply_context_clear(&receipt));
        assert!(model.accepts_idle_turn());
        model.event(json!({"type":"message","message_id":id,"role":"system"}));
        let marker = model.messages[&id].clone();
        assert_eq!(marker.message_subtype.as_deref(), Some("manual_clear_context"));
        model.page(Page { messages: vec![marker], ..Page::default() }, false);
        assert_eq!(model.messages.len(), 2);
    }

    #[test]
    fn queue_snapshot_and_live_events_converge_without_creating_history_rows() {
        let mut model = Model::default();
        let snapshot = || Page {
            execution_context: json!({"queued_user_messages":[{"queued_user_message_id":"a","content":"first"}]}),
            ..Page::default()
        };
        model.page(snapshot(), false);
        assert_eq!(model.queued_messages[0].content, "first");
        let queued = json!({"type":"message","role":"user","message_id":null,"content":"second","metadata":{"queue_status":"queued","queued_user_message_id":"b"}});
        model.event(queued.clone());
        model.event(queued);
        model.page(snapshot(), false);
        assert_eq!(
            model
                .queued_messages
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(model.messages.is_empty());
        model.event(json!({"type":"queued_user_message_removed","queued_user_message_id":"a"}));
        model.page(snapshot(), false);
        assert_eq!(model.queued_messages.len(), 1);
        model.event(json!({"type":"message","role":"user","message_id":"42","content":"second","metadata":{"queue_status":"applied","queued_user_message_id":"b"}}));
        assert!(model.queued_messages.is_empty());
        assert_eq!(model.messages[&42].message, "second");
    }

    #[test]
    fn snapshot_execution_context_tracks_the_waiting_tool() {
        let mut model = Model::default();
        model.page(
            serde_json::from_value(serde_json::json!({
                "status": "awaiting_user",
                "executionContext": {
                    "waitReason": "user_input",
                    "awaitingUserToolCallId": "call-ask",
                },
                "messages": [],
            }))
            .expect("snapshot"),
            false,
        );
        assert_eq!(model.wait_reason, "user_input");
        assert_eq!(
            model.awaiting_user_tool_call_id.as_deref(),
            Some("call-ask")
        );

        // Live events from runtimes that omit the ID must not discard the
        // active question already identified by the snapshot.
        model.event(json!({"type":"state","state":"awaiting_user","wait_reason":"user_input"}));
        assert_eq!(model.awaiting_user_tool_call_id.as_deref(), Some("call-ask"));

        model.event(json!({"type":"state","state":"executing","wait_reason":"approval"}));
        assert_eq!(model.wait_reason, "approval");
        assert_eq!(model.awaiting_user_tool_call_id, None);

        model.event(json!({"type":"state","state":"awaiting_user","wait_reason":"user_input"}));
        assert_eq!(model.awaiting_user_tool_call_id, None);
        model.event(json!({"type":"state","state":"awaiting_user","wait_reason":"user_input",
            "awaiting_user_tool_call_id":"next-question"}));
        assert_eq!(model.awaiting_user_tool_call_id.as_deref(), Some("next-question"));
    }

    #[test]
    fn reading_keeps_accepting_messages_and_follow_returns_to_tail() {
        let mut model = Model::default();
        model.select();
        model.page(Model::demo_page(), false);
        assert!(!model.pause_for_scroll(0.0));
        assert!(!model.pause_for_scroll(-12.0));
        for id in 501..=503 {
            model.event(json!({"type":"message","message_id":id.to_string(),"role":"assistant","content":"latest"}));
            assert!(model.messages.contains_key(&id));
            assert!(!model.reading_history);
        }
        assert!(model.pause_for_scroll(12.0));
        model.event(
            json!({"type":"message","message_id":"504","role":"assistant","content":"background"}),
        );
        let anchor = model.visible().next().map(|m| m.id);
        assert!(model.messages.contains_key(&504));
        model.resume_follow();
        assert!(!model.reading_history);
        assert_eq!(model.visible().last().map(|m| m.id), Some(504));
        assert_eq!(model.visible().next().map(|m| m.id), anchor);
        model.event(json!({"type":"message","message_id":"505","role":"assistant","content":"following again"}));
        assert!(model.messages.contains_key(&505));
    }

    #[test]
    fn final_replaces_delta_and_dedupes_snapshot_id() {
        let mut m = Model::default();
        m.page(
            serde_json::from_value(json!({"messages":[{"id":7,"message":"old"}]})).unwrap(),
            false,
        );
        m.event(json!({"type":"chunk","content":"delta"}));
        m.event(json!({"type":"reasoning_chunk","content":"thought"}));
        assert_eq!(m.live.reasoning.as_deref(), Some("thought"));
        m.event(json!({"type":"message","message_id":"7","role":"assistant","content":"final"}));
        assert_eq!(m.messages.len(), 1);
        assert_eq!(m.messages[&7].message, "final");
        assert!(m.live.message.is_empty());
    }

    #[test]
    fn snapshot_reconcile_preserves_partial_then_recovers_missed_final() {
        let mut m = Model::default();
        m.page(Model::demo_page(), false);
        m.event(json!({"type":"chunk","content":"partial"}));
        m.page(Model::demo_page(), false);
        assert_eq!(m.live.message, "partial");
        let mut page = Model::demo_page();
        let final_id = PAGE_LIMIT as i64 + 1;
        page.messages.push(Message {
            id: final_id,
            role: "assistant".into(),
            message: "final".into(),
            ..Message::default()
        });
        m.page(page, false);
        assert!(m.live.message.is_empty());
        assert_eq!(m.messages[&final_id].message, "final");
        assert_eq!(m.messages.len(), PAGE_LIMIT + 1);
    }

    #[test]
    fn provisional_queue_recovers_from_failure_and_accepts_authoritative_result() {
        let mut model = Model::default();
        let provisional = || QueuedMessage {
            id: "local".into(),
            content: "next".into(),
            metadata: Value::Null,
            provisional: true,
        };
        model.queued_messages.push(provisional());
        model.page(Page::default(), false);
        assert!(model.queued_messages[0].provisional);
        assert!(model.reject_queue_submission("local"));
        assert!(model.queued_messages.is_empty());
        model.queued_messages.push(provisional());
        model.event(json!({"type":"message","role":"user","content":"next","metadata":{"queue_status":"queued","queued_user_message_id":"local"}}));
        assert_eq!(model.queued_messages.len(), 1);
        assert!(!model.queued_messages[0].provisional);
        assert!(!model.reject_queue_submission("local"));
        model.event(json!({"type":"message","role":"user","message_id":"42","content":"next","metadata":{"client_message_id":"local"}}));
        assert!(model.queued_messages.is_empty());
        assert!(model.messages.contains_key(&42));
        let previous = model.generation;
        model.select();
        assert!(!model.accepts(previous));
    }

    #[test]
    fn queue_edit_requires_matching_cancellation_and_ignores_stale_failure() {
        use std::time::Duration;

        let mut model = Model::default();
        let queued = json!({"type":"message", "role":"user", "content":"original",
            "metadata":{"queue_status":"queued", "queued_user_message_id":"old"}});
        let removed = json!({"type":"queued_user_message_removed", "queued_user_message_id":"old",
            "cancelled_before_apply":true});
        model.queued_messages.push(QueuedMessage {
            id: "old".into(), content: "original".into(), metadata: Value::Null, provisional: true,
        });
        assert!(!model.can_remove_queued_message("old"));
        model.event(queued.clone());
        assert!(model.can_remove_queued_message("old"));
        let first = Instant::now();
        model.mark_queue_removal("old", first, true);
        assert!(!model.can_remove_queued_message("old"));
        assert!(!model.reject_queue_removal("other", first));
        assert!(model.reject_queue_removal("old", first));
        assert!(model.can_remove_queued_message("old"));
        let retry = first + Duration::from_secs(1);
        model.mark_queue_removal("old", retry, true);
        assert!(!model.reject_queue_removal("old", first));
        model.page(Page {
            execution_context: json!({"removed_queued_user_message_ids":["old"]}),
            ..Page::default()
        }, false);
        assert!(model.queue_removal_pending("old"));
        assert_eq!(model.take_queue_removal_result("old"), None);
        model.event(removed);
        assert!(!model.queue_removal_pending("old"));
        assert_eq!(model.take_queue_removal_result("old"), Some(true));
        assert!(!model.reject_queue_removal("old", retry));
        model.event(queued);
        assert!(model.queued_messages.is_empty());
    }

    #[test]
    fn queue_edit_does_not_reopen_an_applied_or_unconfirmed_message() {
        let mut model = Model::default();
        let now = Instant::now();
        model.mark_queue_removal("applied", now, true);
        model.event(json!({"type":"message", "role":"user", "message_id":"42",
            "metadata":{"client_message_id":"applied"}}));
        model.event(json!({"type":"queued_user_message_removed",
            "queued_user_message_id":"applied", "cancelled_before_apply":true}));
        assert_eq!(model.take_queue_removal_result("applied"), Some(false));
        assert!(model.messages.contains_key(&42));

        model.mark_queue_removal("late", now, true);
        assert!(model.expire_queue_removals(now + std::time::Duration::from_secs(30)));
        assert!(!model.queue_removal_pending("late"));
        model.event(json!({"type":"queued_user_message_removed",
            "queued_user_message_id":"late", "cancelled_before_apply":true}));
        assert_eq!(model.take_queue_removal_result("late"), None);
    }

    #[test]
    fn switching_rejects_previous_generation() {
        let mut m = Model::default();
        let old = m.select();
        let new = m.select();
        assert!(!m.accepts(old));
        assert!(m.accepts(new));
    }

    #[test]
    fn compatibility_page_dedupes_and_keeps_opaque_metadata() {
        let page: Page = serde_json::from_value(json!({"messages":[
            {"id":2,"message":"old"},{"id":2,"message":"new","metadata":{"MixedKey":1}}],
            "beforeMessageId":"2","hiddenEarlierMessageCount":42}))
        .unwrap();
        let mut m = Model::default();
        m.page(page, true);
        assert_eq!(m.messages.len(), 1);
        assert_eq!(m.messages[&2].metadata["MixedKey"], 1);
        assert_eq!(m.before.as_deref(), Some("2"));
        m.page(Model::demo_page(), false);
        assert!(m.messages.contains_key(&300));
        assert_eq!(m.visible().next().map(|m| m.id), Some(1));
        assert!(m.reading_history);
        m.resume_follow();
        assert_eq!(m.visible().last().map(|m| m.id), Some(PAGE_LIMIT as i64));
    }

    #[test]
    fn frozen_reading_is_stable_and_live_cache_is_bounded() {
        let mut m = Model::default();
        m.page(Model::demo_page(), false);
        m.event(json!({"type":"chunk","content":"visible body"}));
        m.event(json!({"type":"tool_started","tool_call_id":"x","tool_name":"read","arguments":{"path":"a"}}));
        m.freeze();
        m.event(json!({"type":"tool_completed","tool_call_id":"x","result":"ok"}));
        assert_eq!(m.tools["x"]["tool_name"], "read");
        assert_eq!(m.tools["x"]["arguments"]["path"], "a");
        for event in [
            json!({"type":"message","role":"tool","message_id":"301","content":"tool"}),
            json!({"type":"message","role":"user","message_id":"302"}),
            json!({"type":"message","role":"assistant","content":"ephemeral"}),
        ] {
            m.event(event);
            assert_eq!(m.live.message, "visible body");
        }
        m.event(json!({"type":"chunk","content":"x".repeat(LIVE_BYTES_LIMIT)}));
        assert!(m.live_overflow);
        assert!(m.live.message.len() <= LIVE_BYTES_LIMIT);
        let anchor = m.visible().next().map(|m| m.id);
        assert_eq!(m.tools["x"]["type"], "tool_completed");
        m.event(json!({"type":"message","role":"assistant","message_id":"303","content":"final"}));
        assert!(m.live.message.is_empty());
        assert!(m.messages.contains_key(&303));
        assert_eq!(m.visible().next().map(|m| m.id), anchor);
        let mut page = Model::demo_page();
        page.messages.push(Message {
            id: 304,
            role: "assistant".into(),
            message: "snapshot final".into(),
            ..Message::default()
        });
        m.page(page, false);
        assert!(m.messages.contains_key(&304));
        assert_eq!(m.visible().next().map(|m| m.id), anchor);
        for id in 305..=1000 {
            m.event(json!({"type":"message","role":"assistant","message_id":id.to_string(),"content":"new"}));
        }
        assert_eq!(m.visible().next().map(|m| m.id), anchor);
        assert_eq!(m.messages.len(), 1000);
        m.resume_follow();
        assert_eq!(m.visible().last().map(|m| m.id), Some(1000));
        assert_eq!(m.messages.len(), 1000);
    }

    #[test]
    fn older_pages_prepend_without_replacing_loaded_history() {
        let mut model = Model::default();
        model.page(Model::demo_history("session", 1201), false);
        model.page(Model::demo_history("session", 701), true);
        assert_eq!(model.messages.len(), 1000);
        assert_eq!(model.visible().next().map(|row| row.id), Some(201));
        assert_eq!(model.visible().last().map(|row| row.id), Some(1200));
        assert_eq!(model.before.as_deref(), Some("201"));
        model.page(Model::demo_history("session", 1201), false);
        assert_eq!(model.messages.len(), 1000);
        assert_eq!(model.before.as_deref(), Some("201"));
        model.page(Model::demo_history("session", 201), true);
        assert_eq!(model.messages.len(), 1200);
        assert_eq!(model.hidden, 0);
        model.resume_follow();
        assert_eq!(model.messages.len(), 1200);
    }

    #[test]
    fn tools_are_keyed_structurally_and_bounded() {
        let mut m = Model::default();
        for i in 0..100 {
            m.event(json!({"type":"tool_started","tool_call_id":i.to_string()}));
        }
        assert_eq!(m.tools.len(), 32);
        m.event(json!({"type":"tool_completed","tool_call_id":"99","result":{"a":1}}));
        assert_eq!(m.tools["99"]["result"]["a"], 1);
    }
}
