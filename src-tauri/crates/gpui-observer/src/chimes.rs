//! Notification preferences shared with Vue and classification of live events.

use serde_json::Value;
use std::collections::BTreeSet;

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

/// Only reads live events, not history pages or periodically refreshed snapshots.
#[derive(Default)]
pub struct ChimeTracker {
    completed: bool,
    awaiting_user: bool,
    approvals: BTreeSet<(String, String)>,
    waiting_children: BTreeSet<String>,
}

impl ChimeTracker {
    pub fn event(&mut self, event: &Value) -> Option<Chime> {
        match event["type"].as_str()? {
            "confirm" => self.approval("", event["id"].as_str()?),
            "sub_agent_approval_requested" => self.approval(
                event["sub_agent_id"].as_str()?,
                event["tool_call_id"].as_str()?,
            ),
            "approval_resolved" | "sub_agent_approval_resolved" => {
                self.approvals.remove(&(
                    event["sub_agent_id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    event["tool_call_id"].as_str()?.to_owned(),
                ));
                None
            }
            "state" => {
                let state = event["state"].as_str()?;
                let completed = state == "completed";
                let awaiting_user =
                    state == "awaiting_user" || event["wait_reason"] == "user_input";
                let new_completion = completed && !self.completed;
                let new_question = awaiting_user && !self.awaiting_user;
                self.completed = completed;
                self.awaiting_user = awaiting_user;
                if matches!(
                    state,
                    "completed" | "failed" | "cancelled" | "interrupted" | "error"
                ) {
                    self.approvals.clear();
                    self.waiting_children.clear();
                }
                if new_completion {
                    Some(Chime::Completion)
                } else {
                    new_question.then_some(Chime::Approval)
                }
            }
            "sub_agent_progress" => {
                let id = event["sub_agent_id"].as_str()?;
                if event["wait_reason"] == "user_input" {
                    self.waiting_children
                        .insert(id.to_owned())
                        .then_some(Chime::Approval)
                } else {
                    self.waiting_children.remove(id);
                    None
                }
            }
            _ => None,
        }
    }

    fn approval(&mut self, session: &str, id: &str) -> Option<Chime> {
        if id.is_empty() {
            return None;
        }
        self.approvals
            .insert((session.to_owned(), id.to_owned()))
            .then_some(Chime::Approval)
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
        let mut tracker = ChimeTracker::default();
        let confirm = json!({"type":"confirm", "id":"call"});
        let child = json!({"type":"sub_agent_approval_requested", "sub_agent_id":"child", "tool_call_id":"call"});
        let completed = json!({"type":"state", "state":"completed"});
        assert_eq!(tracker.event(&confirm), Some(Chime::Approval));
        assert_eq!(tracker.event(&confirm), None);
        assert_eq!(tracker.event(&child), Some(Chime::Approval));
        assert_eq!(tracker.event(&child), None);
        for state in ["failed", "cancelled", "interrupted", "error"] {
            assert_eq!(tracker.event(&json!({"type":"state", "state":state})), None);
        }
        assert_eq!(tracker.event(&completed), Some(Chime::Completion));
        assert_eq!(tracker.event(&completed), None);
        tracker.event(&json!({"type":"state", "state":"executing"}));
        assert_eq!(tracker.event(&completed), Some(Chime::Completion));
    }

    #[test]
    fn ask_user_reminders_reset_when_the_wait_ends() {
        let mut tracker = ChimeTracker::default();
        let wait = json!({"type":"sub_agent_progress", "sub_agent_id":"child", "wait_reason":"user_input"});
        assert_eq!(tracker.event(&wait), Some(Chime::Approval));
        assert_eq!(tracker.event(&wait), None);
        tracker.event(&json!({"type":"sub_agent_progress", "sub_agent_id":"child"}));
        assert_eq!(tracker.event(&wait), Some(Chime::Approval));
        let parent_wait = json!({"type":"state", "state":"waiting", "wait_reason":"user_input"});
        assert_eq!(tracker.event(&parent_wait), Some(Chime::Approval));
        assert_eq!(tracker.event(&parent_wait), None);
    }
}
