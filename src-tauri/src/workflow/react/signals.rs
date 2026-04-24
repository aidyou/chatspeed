use std::collections::VecDeque;
use std::sync::OnceLock;

use serde_json::Value;

/// Runtime signal classification used by non-blocking signal drains.
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeSignal {
    Stop,
    UserMessage {
        content: String,
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

    if signal_type_enum == Some(SignalType::Stop) || raw.to_lowercase().contains("stop") {
        return RuntimeSignal::Stop;
    }

    if matches!(
        signal_type_enum,
        Some(SignalType::UserMessage | SignalType::LegacyUserInput)
    ) {
        return RuntimeSignal::UserMessage {
            content: parsed["content"].as_str().unwrap_or("").to_string(),
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
    UpdateApprovalLevel,
    UpdateAllowedPaths,
    UpdateModelConfig,
    RemoveShellPolicyItem,
    RemoveAutoApprovedTool,
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
            "update_approval_level" => Some(SignalType::UpdateApprovalLevel),
            "update_allowed_paths" => Some(SignalType::UpdateAllowedPaths),
            "update_model_config" => Some(SignalType::UpdateModelConfig),
            "remove_shell_policy_item" => Some(SignalType::RemoveShellPolicyItem),
            "remove_auto_approved_tool" => Some(SignalType::RemoveAutoApprovedTool),
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
            SignalType::UpdateApprovalLevel => "update_approval_level",
            SignalType::UpdateAllowedPaths => "update_allowed_paths",
            SignalType::UpdateModelConfig => "update_model_config",
            SignalType::RemoveShellPolicyItem => "remove_shell_policy_item",
            SignalType::RemoveAutoApprovedTool => "remove_auto_approved_tool",
        }
    }
}

fn stashed_user_messages() -> &'static dashmap::DashMap<String, VecDeque<(String, String)>> {
    static STASHED: OnceLock<dashmap::DashMap<String, VecDeque<(String, String)>>> =
        OnceLock::new();
    STASHED.get_or_init(dashmap::DashMap::new)
}

fn stashed_runtime_signals() -> &'static dashmap::DashMap<String, VecDeque<String>> {
    static STASHED: OnceLock<dashmap::DashMap<String, VecDeque<String>>> = OnceLock::new();
    STASHED.get_or_init(dashmap::DashMap::new)
}

/// Stores a user message that was observed in a temporary signal consumer (e.g. retry backoff).
pub fn stash_user_message(session_id: &str, queued_id: String, content: String) {
    let mut entry = stashed_user_messages()
        .entry(session_id.to_string())
        .or_default();
    entry.push_back((queued_id, content));
}

/// Stores a runtime signal that must be handled by the workflow engine loop.
pub fn stash_runtime_signal(session_id: &str, signal: String) {
    let mut entry = stashed_runtime_signals()
        .entry(session_id.to_string())
        .or_default();
    entry.push_back(signal);
}

/// Drains all stashed user messages for a session in FIFO order.
pub fn take_stashed_user_messages(session_id: &str) -> Vec<(String, String)> {
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
