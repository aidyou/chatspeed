//! Workflow-window pending decisions and notification preferences shared with Vue.

use serde_json::Value;
use std::collections::BTreeMap;

/// The two independently muted notifications in the workflow settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Chime {
    Approval,
    Completion,
}

impl Chime {
    pub const ALL: [Self; 2] = [Self::Approval, Self::Completion];

    pub fn config_key(self) -> &'static str {
        match self {
            Self::Approval => "workflow_approval_muted",
            Self::Completion => "workflow_completion_muted",
        }
    }

    pub fn muted(self, config: &Value) -> bool {
        config[self.config_key()].as_bool().unwrap_or(true)
    }
}

/// One actionable entry in the titlebar queue. Child events belong to their
/// parent's stream, but the action must be taken in the child's transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingDecision {
    pub target: String,
    pub kind: DecisionKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecisionKind {
    Question,
    Approval(String),
}

/// `BackgroundQuestion` is silent while its owning transcript is selected.
#[derive(Debug, PartialEq, Eq)]
pub enum Reminder {
    Sound(Chime),
    BackgroundQuestion,
}

#[derive(Default)]
pub struct NotificationState {
    completed: bool,
    questions: BTreeMap<String, String>,
    approvals: BTreeMap<(String, String), String>,
}

pub fn terminal(state: &str) -> bool {
    matches!(
        state,
        "completed" | "failed" | "cancelled" | "interrupted" | "error"
    )
}

pub fn waiting_for_user(state: &str, reason: &str) -> bool {
    !terminal(state) && (state == "awaiting_user" || reason == "user_input")
}

fn waiting_for_approval(state: &str, reason: &str) -> bool {
    !terminal(state)
        && (matches!(state, "awaiting_approval" | "awaiting_auto_approval") || reason == "approval")
}

impl NotificationState {
    pub fn handles(event: &Value) -> bool {
        matches!(
            event["type"].as_str(),
            Some(
                "state"
                    | "confirm"
                    | "sub_agent_progress"
                    | "sub_agent_approval_requested"
                    | "sub_agent_approval_resolved"
                    | "approval_resolved"
                    | "tool_started"
                    | "tool_completed"
                    | "tool_failed"
            )
        )
    }

    pub fn pending(&self) -> Vec<PendingDecision> {
        self.questions
            .keys()
            .map(|target| PendingDecision {
                target: target.clone(),
                kind: DecisionKind::Question,
            })
            .chain(
                self.approvals
                    .iter()
                    .map(|((target, _), name)| PendingDecision {
                        target: target.clone(),
                        kind: DecisionKind::Approval(name.clone()),
                    }),
            )
            .collect()
    }

    /// Recover existing waits without replaying approval or completion sounds.
    pub fn snapshot(&mut self, owner: &str, page: &Value) -> Option<Reminder> {
        let context = page
            .get("execution_context")
            .or_else(|| page.get("executionContext"));
        let state = page["workflow"]["status"]
            .as_str()
            .or_else(|| page["status"].as_str())
            .unwrap_or_default();
        let reason = context
            .and_then(|ctx| {
                ctx["wait_reason"]
                    .as_str()
                    .or_else(|| ctx["waitReason"].as_str())
            })
            .unwrap_or_default();
        let question = context
            .and_then(|ctx| {
                ctx["awaiting_user_tool_call_id"]
                    .as_str()
                    .or_else(|| ctx["awaitingUserToolCallId"].as_str())
            })
            .unwrap_or_default();
        let reminder = self.state(owner, state, reason, question, false);
        if waiting_for_approval(state, reason) {
            if let Some(tools) = context.and_then(|ctx| {
                ctx["pending_tools"]
                    .as_array()
                    .or_else(|| ctx["pendingTools"].as_array())
            }) {
                self.approvals.retain(|(target, _), _| target != owner);
                for tool in tools {
                    if let Some(id) = tool["tool_call_id"]
                        .as_str()
                        .or_else(|| tool["toolCallId"].as_str())
                    {
                        self.approval(
                            owner,
                            id,
                            tool["tool_name"]
                                .as_str()
                                .or_else(|| tool["toolName"].as_str())
                                .unwrap_or_default(),
                        );
                    }
                }
            }
            if !self.approvals.keys().any(|(target, _)| target == owner) {
                self.approval(owner, "", "");
            }
        }
        reminder
    }

    pub fn event(&mut self, owner: &str, event: &Value) -> Option<Reminder> {
        let kind = event["type"].as_str()?;
        let child = event["sub_agent_id"].as_str().filter(|id| !id.is_empty());
        let reason = event["wait_reason"].as_str().unwrap_or_default();
        match kind {
            "state" => self.state(
                owner,
                event["state"].as_str()?,
                reason,
                event["awaiting_user_tool_call_id"]
                    .as_str()
                    .unwrap_or_default(),
                true,
            ),
            "sub_agent_progress" => {
                let child = child?;
                let state = event["status"]
                    .as_str()
                    .or_else(|| event["workflow_state"].as_str())
                    .unwrap_or_default();
                if terminal(state) {
                    self.questions.remove(child);
                    self.approvals.retain(|(target, _), _| target != child);
                    return None;
                }
                self.question(child, waiting_for_user(state, reason), "")
                    .then_some(Reminder::Sound(Chime::Approval))
            }
            "confirm" | "sub_agent_approval_requested" => {
                let (target, id) = if kind == "confirm" {
                    (owner, event["id"].as_str()?)
                } else {
                    (child?, event["tool_call_id"].as_str()?)
                };
                (!id.is_empty()
                    && self.approval(target, id, event["tool_name"].as_str().unwrap_or_default()))
                .then_some(Reminder::Sound(Chime::Approval))
            }
            "approval_resolved"
            | "sub_agent_approval_resolved"
            | "tool_started"
            | "tool_completed"
            | "tool_failed" => {
                let target = if kind == "sub_agent_approval_resolved" {
                    child?
                } else {
                    owner
                };
                let id = event["tool_call_id"].as_str()?;
                self.approvals.remove(&(target.to_owned(), id.to_owned()));
                self.approvals.remove(&(target.to_owned(), String::new()));
                None
            }
            _ => None,
        }
    }

    fn state(
        &mut self,
        owner: &str,
        state: &str,
        reason: &str,
        question: &str,
        live: bool,
    ) -> Option<Reminder> {
        let completed = state == "completed";
        let new_completion = completed && !self.completed;
        self.completed = completed;
        if terminal(state) {
            self.questions.clear();
            self.approvals.clear();
            return (live && new_completion).then_some(Reminder::Sound(Chime::Completion));
        }
        if !waiting_for_approval(state, reason) {
            self.approvals.retain(|(target, _), _| target != owner);
        }
        self.question(owner, waiting_for_user(state, reason), question)
            .then_some(Reminder::BackgroundQuestion)
    }

    fn question(&mut self, target: &str, waiting: bool, id: &str) -> bool {
        if !waiting {
            self.questions.remove(target);
            return false;
        }
        let previous = self.questions.get(target);
        let new_wait = previous.is_none_or(|old| !old.is_empty() && !id.is_empty() && old != id);
        if previous.is_none() || !id.is_empty() {
            self.questions.insert(target.to_owned(), id.to_owned());
        }
        new_wait
    }

    fn approval(&mut self, target: &str, id: &str, name: &str) -> bool {
        if !id.is_empty() {
            self.approvals.remove(&(target.to_owned(), String::new()));
        }
        let key = (target.to_owned(), id.to_owned());
        let new = !self.approvals.contains_key(&key);
        if new || !name.is_empty() {
            self.approvals.insert(key, name.to_owned());
        }
        new
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn uses_the_vue_defaults_and_independent_config_keys() {
        assert!(Chime::ALL.into_iter().all(|chime| chime.muted(&json!({}))));
        let config = json!({"workflow_approval_muted": false, "workflow_completion_muted": true});
        assert!(!Chime::Approval.muted(&config));
        assert!(Chime::Completion.muted(&config));
    }

    #[test]
    fn new_approvals_and_successful_runs_ring_once() {
        let mut tracker = NotificationState::default();
        let approval = Some(Reminder::Sound(Chime::Approval));
        let completion = Some(Reminder::Sound(Chime::Completion));
        let confirm = json!({"type":"confirm", "id":"call"});
        let child = json!({"type":"sub_agent_approval_requested", "sub_agent_id":"child", "tool_call_id":"call"});
        let completed = json!({"type":"state", "state":"completed"});
        assert_eq!(tracker.event("root", &confirm), approval);
        assert_eq!(tracker.event("root", &confirm), None);
        assert_eq!(tracker.event("root", &child), approval);
        assert_eq!(tracker.event("root", &child), None);
        assert_eq!(tracker.pending().len(), 2);
        tracker.event("root", &json!({"type":"sub_agent_approval_resolved","sub_agent_id":"child","tool_call_id":"call"}));
        assert_eq!(tracker.pending().len(), 1);
        for state in ["failed", "cancelled", "interrupted", "error"] {
            assert_eq!(
                tracker.event("root", &json!({"type":"state", "state":state})),
                None
            );
            assert!(tracker.pending().is_empty());
        }
        assert_eq!(tracker.event("root", &completed), completion);
        assert_eq!(tracker.event("root", &completed), None);
        tracker.event("root", &json!({"type":"state", "state":"executing"}));
        assert_eq!(tracker.event("root", &completed), completion);
    }

    #[test]
    fn ask_user_reminders_reset_when_the_wait_ends() {
        let mut tracker = NotificationState::default();
        let wait = json!({"type":"sub_agent_progress", "sub_agent_id":"child", "wait_reason":"user_input"});
        let approval = Some(Reminder::Sound(Chime::Approval));
        assert_eq!(tracker.event("root", &wait), approval);
        assert_eq!(tracker.pending()[0].target, "child");
        assert_eq!(tracker.event("root", &wait), None);
        tracker.event(
            "root",
            &json!({"type":"sub_agent_progress", "sub_agent_id":"child"}),
        );
        assert!(tracker.pending().is_empty());
        assert_eq!(tracker.event("root", &wait), approval);
        let parent_wait = json!({"type":"state", "state":"waiting", "wait_reason":"user_input"});
        assert_eq!(
            tracker.event("root", &parent_wait),
            Some(Reminder::BackgroundQuestion)
        );
        assert_eq!(tracker.event("root", &parent_wait), None);
        assert_eq!(tracker.pending().len(), 2);
    }

    #[test]
    fn snapshot_recovers_a_pending_question_without_reannouncing_it_on_live_delivery() {
        let mut tracker = NotificationState::default();
        let page = json!({"workflow":{"status":"awaiting_user"},"executionContext":{"waitReason":"user_input","awaitingUserToolCallId":"q1"}});
        assert_eq!(
            tracker.snapshot("root", &page),
            Some(Reminder::BackgroundQuestion)
        );
        assert_eq!(
            tracker.pending(),
            vec![PendingDecision {
                target: "root".into(),
                kind: DecisionKind::Question
            }]
        );
        assert_eq!(
            tracker.event("root", &json!({"type":"state","state":"awaiting_user"})),
            None
        );
        assert_eq!(tracker.snapshot("root", &page), None);
        assert_eq!(
            tracker.snapshot("root", &json!({"workflow":{"status":"completed"}})),
            None
        );
        assert!(tracker.pending().is_empty());
    }
}
