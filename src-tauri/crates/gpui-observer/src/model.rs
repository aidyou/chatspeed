//! Durable-id reconciliation with explicitly loaded, prepended history pages.
use crate::types::{Message, Page};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const PAGE_LIMIT: usize = 300;
pub const LIVE_BYTES_LIMIT: usize = 1024 * 1024;

/// One selected session; history grows only through explicit pagination and live events.
#[derive(Default)]
pub struct Model {
    pub generation: u64,
    pub messages: BTreeMap<i64, Message>,
    pub live: Message,
    pub tools: BTreeMap<String, Value>,
    pub tool_order: Vec<String>,
    pub before: Option<String>,
    pub hidden: usize,
    pub reading_history: bool,
    pub live_overflow: bool,
    pub state: String,
    /// Backend-owned compression hint, mirrored from `compression_status` events.
    pub compressing: bool,
    pub compression_message: String,
    /// Runtime wait reason (`approval`, `user_input`, `confirmation`, `sub_agent`).
    pub wait_reason: String,
    /// Tool call the runtime waits on for an `ask_user` answer.
    pub awaiting_user_tool_call_id: Option<String>,
    /// Tool calls the runtime lists as awaiting a decision.
    pub pending_tool_ids: BTreeSet<String>,
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

    pub fn accepts(&self, generation: u64) -> bool {
        generation == self.generation
    }

    pub fn page(&mut self, mut page: Page, older: bool) {
        if let Some(status) = page.workflow["status"].as_str() {
            page.status = status.to_owned();
        }
        if !page.status.is_empty() {
            self.state = page.status.clone();
        }
        let context = &page.execution_context;
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
        if !older
            && (finalized_live
                || matches!(
                    page.status.as_str(),
                    "completed" | "failed" | "cancelled" | "error"
                ))
        {
            self.live = Message::default();
            self.live_overflow = false;
        }
        if older || initial {
            self.before = page.message_window_before_id;
            self.hidden = page.hidden_earlier_message_count;
        }
        // Snapshot windows are partial: never replace already loaded history.
        self.messages
            .extend(page.messages.into_iter().map(|m| (m.id, m)));
        if older {
            self.reading_history = true;
        }
        if initial {
            self.live = Message::default();
            self.tools.clear();
            self.tool_order.clear();
        } else {
            for row in self.messages.values() {
                if let Some(id) = row.metadata["tool_call_id"].as_str() {
                    self.tools.remove(id);
                }
            }
            self.tool_order.retain(|id| self.tools.contains_key(id));
        }
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

    pub fn reset_live(&mut self) {
        self.live = Message::default();
        self.tools.clear();
        self.tool_order.clear();
        self.live_overflow = false;
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
            "chunk" | "reasoning_chunk" => self.delta(
                event["content"].as_str().unwrap_or_default(),
                kind == "reasoning_chunk",
            ),
            "message" => {
                if let Some(id) = event["message_id"]
                    .as_str()
                    .and_then(|s| s.parse::<i64>().ok())
                {
                    let row = Message {
                        id,
                        role: event["role"].as_str().unwrap_or_default().to_owned(),
                        message: event["content"].as_str().unwrap_or_default().to_owned(),
                        reasoning: event["reasoning"].as_str().map(str::to_owned),
                        metadata: event["metadata"].clone(),
                        ..Message::default()
                    };
                    if let Some(tool) = row.metadata["tool_call_id"].as_str() {
                        self.tools.remove(tool);
                    }
                    if row.role == "assistant" {
                        self.live = Message::default();
                        self.live_overflow = false;
                    }
                    self.messages.insert(id, row);
                }
            }
            "tool_started" | "tool_completed" | "tool_failed" => {
                if let Some(id) = event["tool_call_id"].as_str() {
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
                }
            }
            "state" => {
                self.state = event["state"].as_str().unwrap_or_default().to_owned();
                if let Some(reason) = event["wait_reason"].as_str() {
                    self.wait_reason = reason.to_owned();
                }
            }
            "compression_status" => {
                self.compressing = event["is_compressing"].as_bool().unwrap_or(false);
                self.compression_message = event["message"].as_str().unwrap_or_default().to_owned();
            }
            "compression_applied" => {
                self.compressing = false;
                self.compression_message.clear();
            }
            _ => {}
        }
    }

    pub fn demo_history(session: &str, before: i64) -> Page {
        let mut page = Self::demo_page();
        let end = (before - 1).max(0);
        let start = (end - 299).max(1);
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
            messages: (1..=300).map(|id| Message {
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

        model.event(json!({"type":"state","state":"executing","wait_reason":"approval"}));
        assert_eq!(model.wait_reason, "approval");
    }

    #[test]
    fn reading_keeps_accepting_messages_and_follow_returns_to_tail() {
        let mut model = Model::default();
        model.select();
        model.page(Model::demo_page(), false);
        assert!(!model.pause_for_scroll(0.0));
        assert!(!model.pause_for_scroll(-12.0));
        for id in 301..=303 {
            model.event(json!({"type":"message","message_id":id.to_string(),"role":"assistant","content":"latest"}));
            assert!(model.messages.contains_key(&id));
            assert!(!model.reading_history);
        }
        assert!(model.pause_for_scroll(12.0));
        model.event(
            json!({"type":"message","message_id":"304","role":"assistant","content":"background"}),
        );
        let anchor = model.visible().next().map(|m| m.id);
        assert!(model.messages.contains_key(&304));
        model.resume_follow();
        assert!(!model.reading_history);
        assert_eq!(model.visible().last().map(|m| m.id), Some(304));
        assert_eq!(model.visible().next().map(|m| m.id), anchor);
        model.event(json!({"type":"message","message_id":"305","role":"assistant","content":"following again"}));
        assert!(model.messages.contains_key(&305));
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
        page.messages.push(Message {
            id: 301,
            role: "assistant".into(),
            message: "final".into(),
            ..Message::default()
        });
        m.page(page, false);
        assert!(m.live.message.is_empty());
        assert_eq!(m.messages[&301].message, "final");
        assert_eq!(m.messages.len(), PAGE_LIMIT + 1);
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
        assert_eq!(m.visible().last().map(|m| m.id), Some(300));
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
    fn older_pages_prepend_three_hundred_without_replacing_loaded_history() {
        let mut model = Model::default();
        let mut latest = Model::demo_page();
        for row in &mut latest.messages {
            row.id += 600;
        }
        latest.message_window_before_id = Some("601".into());
        latest.hidden_earlier_message_count = 600;
        model.page(latest, false);
        model.page(Model::demo_history("session", 601), true);
        assert_eq!(model.messages.len(), 600);
        assert_eq!(model.visible().next().map(|row| row.id), Some(301));
        assert_eq!(model.visible().last().map(|row| row.id), Some(900));
        assert_eq!(model.before.as_deref(), Some("301"));
        let mut snapshot = Model::demo_page();
        for row in &mut snapshot.messages {
            row.id += 600;
        }
        model.page(snapshot, false);
        assert_eq!(model.messages.len(), 600);
        assert_eq!(model.before.as_deref(), Some("301"));
        model.page(Model::demo_history("session", 301), true);
        assert_eq!(model.messages.len(), 900);
        assert_eq!(model.hidden, 0);
        model.resume_follow();
        assert_eq!(model.messages.len(), 900);
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
