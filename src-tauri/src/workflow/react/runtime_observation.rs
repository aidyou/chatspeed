//! Typed runtime observations and projection helpers.

use crate::db::WorkflowMessage;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const MESSAGE_KIND_RUNTIME_OBSERVATION: &str = "runtime_observation";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeObservationType {
    SubAgentCompletion,
    SubAgentInterrupted,
    CompletionRejected,
    ActiveTodosBlocked,
    AuditRejected,
    NoToolCall,
    InvalidToolCall,
    LoopDetected,
    TurnBlockedPostponed,
    StepBudgetWarning,
    SkillActivated,
    FileContextAttached,
    GenericReminder,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeObservationLlmVisibility {
    PreservePosition,
    Defer,
    Hide,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeObservationUiVisibility {
    Show,
    Hide,
    Card,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeObservationPlacement {
    Preserve,
    Defer,
    Hide,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedObservation {
    pub content: String,
    pub placement: RuntimeObservationPlacement,
}

pub fn runtime_observation_metadata(
    observation_type: RuntimeObservationType,
    data: Value,
) -> Value {
    let (llm_visibility, ui_visibility) = default_visibility(observation_type);
    runtime_observation_metadata_with_visibility(
        observation_type,
        llm_visibility,
        ui_visibility,
        data,
    )
}

pub fn runtime_observation_metadata_with_visibility(
    observation_type: RuntimeObservationType,
    llm_visibility: RuntimeObservationLlmVisibility,
    ui_visibility: RuntimeObservationUiVisibility,
    data: Value,
) -> Value {
    json!({
        "message_kind": MESSAGE_KIND_RUNTIME_OBSERVATION,
        "observation_type": observation_type,
        "llm_visibility": llm_visibility,
        "ui_visibility": ui_visibility,
        "data": data,
    })
}

pub fn enrich_runtime_observation_metadata(
    metadata: &mut Value,
    observation_type: RuntimeObservationType,
    data: Value,
) {
    let typed = runtime_observation_metadata(observation_type, data);
    merge_object_fields(metadata, typed);
}

pub fn is_runtime_observation(metadata: Option<&Value>) -> bool {
    metadata
        .and_then(|meta| meta.get("message_kind"))
        .and_then(Value::as_str)
        == Some(MESSAGE_KIND_RUNTIME_OBSERVATION)
}

pub fn runtime_observation_type(metadata: Option<&Value>) -> Option<RuntimeObservationType> {
    let value = metadata?.get("observation_type")?.clone();
    serde_json::from_value(value).ok()
}

pub fn render_runtime_observation_for_llm(
    message: &WorkflowMessage,
) -> Option<RenderedObservation> {
    let metadata = message.metadata.as_ref();
    let is_typed = is_runtime_observation(metadata);
    let is_legacy_system_observation = is_legacy_system_observation(message);

    if !is_typed && !is_legacy_system_observation {
        return None;
    }

    let content = metadata
        .and_then(|meta| meta.get("data"))
        .and_then(|data| data.get("llm_content"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| message.message.clone());

    let observation_type = runtime_observation_type(metadata);
    // Function-calling history must retain the result paired with every tool call.
    let is_tool_result = message.role == "tool"
        && metadata.is_some_and(|metadata| {
            metadata
                .get("tool_call_id")
                .or_else(|| {
                    metadata
                        .get("data")
                        .and_then(|data| data.get("tool_call_id"))
                })
                .and_then(Value::as_str)
                .is_some_and(|tool_call_id| !tool_call_id.trim().is_empty())
        });
    let visibility = metadata
        .and_then(|meta| meta.get("llm_visibility"))
        .and_then(|value| {
            serde_json::from_value::<RuntimeObservationLlmVisibility>(value.clone()).ok()
        });
    let placement = if is_tool_result
        || observation_type == Some(RuntimeObservationType::SubAgentCompletion)
        || is_legacy_sub_agent_completion_observation(message)
    {
        RuntimeObservationPlacement::Preserve
    } else if observation_type == Some(RuntimeObservationType::SubAgentInterrupted)
        && visibility == Some(RuntimeObservationLlmVisibility::Defer)
    {
        RuntimeObservationPlacement::Preserve
    } else {
        visibility
            .map(llm_visibility_to_placement)
            .unwrap_or(RuntimeObservationPlacement::Defer)
    };

    Some(RenderedObservation { content, placement })
}

fn is_legacy_system_observation(message: &WorkflowMessage) -> bool {
    message.role == "user"
        && message.step_type.as_deref() == Some("observe")
        && message.message.contains("<SYSTEM_REMINDER>")
}

fn is_legacy_sub_agent_completion_observation(message: &WorkflowMessage) -> bool {
    is_legacy_system_observation(message) && message.message.contains("<tool_result")
}

fn default_visibility(
    observation_type: RuntimeObservationType,
) -> (
    RuntimeObservationLlmVisibility,
    RuntimeObservationUiVisibility,
) {
    match observation_type {
        RuntimeObservationType::SubAgentCompletion => (
            RuntimeObservationLlmVisibility::PreservePosition,
            RuntimeObservationUiVisibility::Card,
        ),
        RuntimeObservationType::SubAgentInterrupted => (
            RuntimeObservationLlmVisibility::PreservePosition,
            RuntimeObservationUiVisibility::Hide,
        ),
        RuntimeObservationType::AuditRejected
        | RuntimeObservationType::CompletionRejected
        | RuntimeObservationType::ActiveTodosBlocked => (
            RuntimeObservationLlmVisibility::PreservePosition,
            RuntimeObservationUiVisibility::Show,
        ),
        RuntimeObservationType::SkillActivated | RuntimeObservationType::FileContextAttached => (
            RuntimeObservationLlmVisibility::PreservePosition,
            RuntimeObservationUiVisibility::Hide,
        ),
        _ => (
            RuntimeObservationLlmVisibility::Defer,
            RuntimeObservationUiVisibility::Hide,
        ),
    }
}

fn llm_visibility_to_placement(
    visibility: RuntimeObservationLlmVisibility,
) -> RuntimeObservationPlacement {
    match visibility {
        RuntimeObservationLlmVisibility::PreservePosition => RuntimeObservationPlacement::Preserve,
        RuntimeObservationLlmVisibility::Defer => RuntimeObservationPlacement::Defer,
        RuntimeObservationLlmVisibility::Hide => RuntimeObservationPlacement::Hide,
    }
}

fn merge_object_fields(target: &mut Value, source: Value) {
    if let (Some(target), Some(source)) = (target.as_object_mut(), source.as_object()) {
        for (key, value) in source {
            target.insert(key.clone(), value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::WorkflowMessage;

    #[test]
    fn runtime_observation_metadata_has_stable_shape() {
        let metadata = runtime_observation_metadata(
            RuntimeObservationType::SubAgentCompletion,
            json!({ "sub_agent_id": "subagent_1" }),
        );

        assert_eq!(
            metadata["message_kind"].as_str(),
            Some(MESSAGE_KIND_RUNTIME_OBSERVATION)
        );
        assert_eq!(
            metadata["observation_type"].as_str(),
            Some("sub_agent_completion")
        );
        assert_eq!(
            metadata["llm_visibility"].as_str(),
            Some("preserve_position")
        );
        assert_eq!(metadata["ui_visibility"].as_str(), Some("card"));
        assert_eq!(
            metadata["data"]["sub_agent_id"].as_str(),
            Some("subagent_1")
        );
    }

    #[test]
    fn sub_agent_interrupted_metadata_preserves_position_and_stays_hidden_in_ui() {
        let metadata = runtime_observation_metadata(
            RuntimeObservationType::SubAgentInterrupted,
            json!({ "sub_agent_id": "subagent_1" }),
        );

        assert_eq!(
            metadata["llm_visibility"].as_str(),
            Some("preserve_position")
        );
        assert_eq!(metadata["ui_visibility"].as_str(), Some("hide"));
    }

    #[test]
    fn sub_agent_interrupted_observation_preserves_position_for_legacy_defer_metadata() {
        let mut metadata = runtime_observation_metadata(
            RuntimeObservationType::SubAgentInterrupted,
            json!({ "sub_agent_id": "subagent_1" }),
        );
        metadata["llm_visibility"] = json!("defer");
        let message = WorkflowMessage {
            id: None,
            session_id: "session".to_string(),
            role: "user".to_string(),
            message: "<SYSTEM_REMINDER>Sub-agent interrupted.</SYSTEM_REMINDER>".to_string(),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: Some("sub_agent_interrupted".to_string()),
            metadata: Some(metadata),
            attached_context: None,
            step_type: Some("observe".to_string()),
            step_index: 0,
            is_error: true,
            error_type: Some("SubAgentInterrupted".to_string()),
            created_at: None,
        };

        let rendered = render_runtime_observation_for_llm(&message)
            .expect("sub-agent interruption should render");
        assert_eq!(rendered.placement, RuntimeObservationPlacement::Preserve);
    }

    #[test]
    fn sub_agent_interrupted_observation_respects_explicit_hide_metadata() {
        let mut metadata = runtime_observation_metadata(
            RuntimeObservationType::SubAgentInterrupted,
            json!({ "sub_agent_id": "subagent_1" }),
        );
        metadata["llm_visibility"] = json!("hide");
        let message = WorkflowMessage {
            id: None,
            session_id: "session".to_string(),
            role: "user".to_string(),
            message: "<SYSTEM_REMINDER>Sub-agent interrupted.</SYSTEM_REMINDER>".to_string(),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: Some("sub_agent_interrupted".to_string()),
            metadata: Some(metadata),
            attached_context: None,
            step_type: Some("observe".to_string()),
            step_index: 0,
            is_error: true,
            error_type: Some("SubAgentInterrupted".to_string()),
            created_at: None,
        };

        let rendered = render_runtime_observation_for_llm(&message)
            .expect("hidden sub-agent interruption should still normalize");
        assert_eq!(rendered.placement, RuntimeObservationPlacement::Hide);
    }

    #[test]
    fn legacy_sub_agent_completion_observation_preserves_position() {
        let message = WorkflowMessage {
            id: None,
            session_id: "session".to_string(),
            role: "user".to_string(),
            message: "<tool_result tool=\"sub_agent_run\" id=\"subagent_1\" mode=\"call\" status=\"completed\">\n<Result>\nDone\n</Result>\n</tool_result>\n<SYSTEM_REMINDER>Use the result.</SYSTEM_REMINDER>".to_string(),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: None,
            metadata: None,
            attached_context: None,
            step_type: Some("observe".to_string()),
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        };

        let rendered = render_runtime_observation_for_llm(&message)
            .expect("legacy sub-agent completion should still render");
        assert_eq!(rendered.placement, RuntimeObservationPlacement::Preserve);
    }

    #[test]
    fn deferred_loop_detected_tool_result_preserves_function_call_pairing() {
        let mut metadata = runtime_observation_metadata(
            RuntimeObservationType::LoopDetected,
            json!({
                "tool_call_id": "fc_complete_1",
                "llm_content": "<SYSTEM_REMINDER>Use a non-empty summary.</SYSTEM_REMINDER>"
            }),
        );
        metadata["tool_call_id"] = json!("fc_complete_1");
        let message = WorkflowMessage {
            id: None,
            session_id: "session".to_string(),
            role: "tool".to_string(),
            message: "Loop detected".to_string(),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: None,
            metadata: Some(metadata),
            attached_context: None,
            step_type: Some("observe".to_string()),
            step_index: 2,
            is_error: true,
            error_type: Some("LoopDetected".to_string()),
            created_at: None,
        };

        let rendered = render_runtime_observation_for_llm(&message)
            .expect("loop-detected tool result should render");
        assert_eq!(rendered.placement, RuntimeObservationPlacement::Preserve);
        assert!(rendered.content.contains("non-empty summary"));
    }
}
