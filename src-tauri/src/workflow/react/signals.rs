use std::collections::{HashSet, VecDeque};
use std::sync::OnceLock;

use serde_json::Value;

/// Runtime signal classification used by non-blocking signal drains.
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeSignal {
    Stop,
    UserMessage {
        content: String,
        attached_context: Option<String>,
        metadata: Option<serde_json::Value>,
        queued_user_message_id: Option<String>,
    },
    Other {
        signal: Option<crate::workflow::react::types::WorkflowSignal>,
        signal_type: Option<SignalType>,
    },
}

/// Parses a raw signal JSON payload into a runtime classification.
pub fn parse_runtime_signal(raw: &str) -> RuntimeSignal {
    let parsed: Value = serde_json::from_str(raw).unwrap_or_default();
    let signal_type = parsed["type"].as_str().unwrap_or_default();
    let signal_type_enum = SignalType::from_str(signal_type);
    let workflow_signal = crate::workflow::react::types::WorkflowSignal::parse(raw);

    if signal_type_enum == Some(SignalType::Stop) || raw.trim().eq_ignore_ascii_case("stop") {
        return RuntimeSignal::Stop;
    }

    if matches!(
        signal_type_enum,
        Some(SignalType::UserMessage | SignalType::LegacyUserInput)
    ) {
        return RuntimeSignal::UserMessage {
            content: parsed["content"].as_str().unwrap_or("").to_string(),
            attached_context: parsed["attached_context"]
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| parsed["attachedContext"].as_str().map(|s| s.to_string())),
            metadata: parsed.get("metadata").cloned(),
            queued_user_message_id: parsed["queued_user_message_id"]
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| {
                    parsed["queuedUserMessageId"]
                        .as_str()
                        .map(|s| s.to_string())
                }),
        };
    }

    RuntimeSignal::Other {
        signal: workflow_signal,
        signal_type: signal_type_enum,
    }
}

/// Canonical signal types used by the workflow runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalType {
    UserMessage,
    LegacyUserInput,
    Approval,
    Continue,
    Stop,
    SubAgentComplete,
    CompressionReady,
    CompressionFailed,
    RebroadcastPending,
    LegacyRequestConfirmBroadcast,
    UpdateFinalAudit,
    UpdateAutoCompress,
    UpdateApprovalLevel,
    UpdatePhase,
    UpdateAllowedPaths,
    UpdateModelConfig,
    UpdateSkillsConfig,
    UpdateAutoApprovedTools,
    RemoveShellPolicyItem,
    RemoveAutoApprovedTool,
    RemoveQueuedUserMessage,
}

impl SignalType {
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "user_message" => Some(SignalType::UserMessage),
            "user_input" => Some(SignalType::LegacyUserInput),
            "approval" => Some(SignalType::Approval),
            "continue" => Some(SignalType::Continue),
            "stop" => Some(SignalType::Stop),
            "sub_agent_complete" => Some(SignalType::SubAgentComplete),
            "compression_ready" => Some(SignalType::CompressionReady),
            "compression_failed" => Some(SignalType::CompressionFailed),
            "rebroadcast_pending" => Some(SignalType::RebroadcastPending),
            "request_confirm_broadcast" => Some(SignalType::LegacyRequestConfirmBroadcast),
            "update_final_audit" => Some(SignalType::UpdateFinalAudit),
            "update_auto_compress" => Some(SignalType::UpdateAutoCompress),
            "update_approval_level" => Some(SignalType::UpdateApprovalLevel),
            "update_phase" => Some(SignalType::UpdatePhase),
            "update_allowed_paths" => Some(SignalType::UpdateAllowedPaths),
            "update_model_config" => Some(SignalType::UpdateModelConfig),
            "update_skills_config" => Some(SignalType::UpdateSkillsConfig),
            "update_auto_approved_tools" => Some(SignalType::UpdateAutoApprovedTools),
            "remove_shell_policy_item" => Some(SignalType::RemoveShellPolicyItem),
            "remove_auto_approved_tool" => Some(SignalType::RemoveAutoApprovedTool),
            "remove_queued_user_message" => Some(SignalType::RemoveQueuedUserMessage),
            _ => None,
        }
    }

    /// Canonical wire name used for logs/docs.
    pub fn as_str(self) -> &'static str {
        match self {
            SignalType::UserMessage => "user_message",
            SignalType::LegacyUserInput => "user_input",
            SignalType::Approval => "approval",
            SignalType::Continue => "continue",
            SignalType::Stop => "stop",
            SignalType::SubAgentComplete => "sub_agent_complete",
            SignalType::CompressionReady => "compression_ready",
            SignalType::CompressionFailed => "compression_failed",
            SignalType::RebroadcastPending => "rebroadcast_pending",
            SignalType::LegacyRequestConfirmBroadcast => "request_confirm_broadcast",
            SignalType::UpdateFinalAudit => "update_final_audit",
            SignalType::UpdateAutoCompress => "update_auto_compress",
            SignalType::UpdateApprovalLevel => "update_approval_level",
            SignalType::UpdatePhase => "update_phase",
            SignalType::UpdateAllowedPaths => "update_allowed_paths",
            SignalType::UpdateModelConfig => "update_model_config",
            SignalType::UpdateSkillsConfig => "update_skills_config",
            SignalType::UpdateAutoApprovedTools => "update_auto_approved_tools",
            SignalType::RemoveShellPolicyItem => "remove_shell_policy_item",
            SignalType::RemoveAutoApprovedTool => "remove_auto_approved_tool",
            SignalType::RemoveQueuedUserMessage => "remove_queued_user_message",
        }
    }
}

fn stashed_user_messages() -> &'static dashmap::DashMap<
    String,
    VecDeque<(String, String, Option<String>, Option<serde_json::Value>)>,
> {
    static STASHED: OnceLock<
        dashmap::DashMap<
            String,
            VecDeque<(String, String, Option<String>, Option<serde_json::Value>)>,
        >,
    > = OnceLock::new();
    STASHED.get_or_init(dashmap::DashMap::new)
}

fn removed_stashed_user_message_ids() -> &'static dashmap::DashMap<String, HashSet<String>> {
    static REMOVED: OnceLock<dashmap::DashMap<String, HashSet<String>>> = OnceLock::new();
    REMOVED.get_or_init(dashmap::DashMap::new)
}

fn stashed_runtime_signals() -> &'static dashmap::DashMap<String, VecDeque<String>> {
    static STASHED: OnceLock<dashmap::DashMap<String, VecDeque<String>>> = OnceLock::new();
    STASHED.get_or_init(dashmap::DashMap::new)
}

/// Stores a user message that was observed in a temporary signal consumer (e.g. retry backoff).
/// Returns whether this was a new queued ID for the session.
pub fn stash_user_message(
    session_id: &str,
    queued_id: String,
    content: String,
    attached_context: Option<String>,
    metadata: Option<serde_json::Value>,
) -> bool {
    if removed_stashed_user_message_ids()
        .get(session_id)
        .is_some_and(|removed_ids| removed_ids.contains(&queued_id))
    {
        return false;
    }

    let mut entry = stashed_user_messages()
        .entry(session_id.to_string())
        .or_default();
    if entry.iter().any(|(id, _, _, _)| id == &queued_id) {
        return false;
    }
    entry.push_back((queued_id, content, attached_context, metadata));
    true
}

/// Restores durable queue-removal tombstones before retry signal consumers become active.
pub fn restore_stashed_user_message_tombstones(session_id: &str, queued_ids: &[String]) {
    let mut removed_ids = removed_stashed_user_message_ids()
        .entry(session_id.to_string())
        .or_default();
    removed_ids.extend(queued_ids.iter().cloned());
}

/// Removes a user message captured by a temporary signal consumer.
pub fn remove_stashed_user_message(session_id: &str, queued_id: &str) -> bool {
    removed_stashed_user_message_ids()
        .entry(session_id.to_string())
        .or_default()
        .insert(queued_id.to_string());
    let Some(mut entry) = stashed_user_messages().get_mut(session_id) else {
        return false;
    };
    let before = entry.len();
    entry.retain(|(id, _, _, _)| id != queued_id);
    before != entry.len()
}

/// Stores a runtime signal that must be handled by the workflow engine loop.
pub fn stash_runtime_signal(session_id: &str, signal: String) {
    let mut entry = stashed_runtime_signals()
        .entry(session_id.to_string())
        .or_default();
    entry.push_back(signal);
}

/// Drains all stashed user messages for a session in FIFO order.
pub fn take_stashed_user_messages(
    session_id: &str,
) -> Vec<(String, String, Option<String>, Option<serde_json::Value>)> {
    if let Some((_, mut queue)) = stashed_user_messages().remove(session_id) {
        let mut drained = Vec::new();
        while let Some(msg) = queue.pop_front() {
            drained.push(msg);
        }
        return drained;
    }
    Vec::new()
}

/// Drains runtime signals captured by temporary signal consumers in FIFO order.
pub fn take_stashed_runtime_signals(session_id: &str) -> Vec<String> {
    if let Some((_, mut queue)) = stashed_runtime_signals().remove(session_id) {
        let mut drained = Vec::new();
        while let Some(signal) = queue.pop_front() {
            drained.push(signal);
        }
        return drained;
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::{
        remove_stashed_user_message, restore_stashed_user_message_tombstones, stash_user_message,
        take_stashed_user_messages,
    };

    #[test]
    fn stashed_user_messages_are_deduplicated_and_removable() {
        let session_id = "stashed-user-message-test";
        let queued_id = "queue-1".to_string();
        assert!(stash_user_message(
            session_id,
            queued_id.clone(),
            "first delivery".to_string(),
            None,
            None,
        ));
        assert!(!stash_user_message(
            session_id,
            queued_id.clone(),
            "duplicate delivery".to_string(),
            None,
            None,
        ));
        assert!(remove_stashed_user_message(session_id, &queued_id));
        assert!(!stash_user_message(
            session_id,
            queued_id,
            "redelivered after removal".to_string(),
            None,
            None,
        ));
        assert!(take_stashed_user_messages(session_id).is_empty());
    }

    #[test]
    fn restored_tombstone_rejects_retry_redelivery() {
        let session_id = "restored-stashed-user-message-test";
        let queued_id = "queue-restored".to_string();
        restore_stashed_user_message_tombstones(session_id, std::slice::from_ref(&queued_id));

        assert!(!stash_user_message(
            session_id,
            queued_id,
            "redelivered after executor rebuild".to_string(),
            None,
            None,
        ));
        assert!(take_stashed_user_messages(session_id).is_empty());
    }
}
