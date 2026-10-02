//! Chat and model wire contracts for the runtime control plane.
//!
//! The standalone runtime owns model listing, chat execution and chat
//! cancellation. The desktop client and the `cs` CLI reach those operations only
//! through the typed `/control/v1` routes declared here, so this module is the
//! single source of truth for their JSON names.
//!
//! Two naming rules apply, matching the rest of the control plane:
//! - requests are canonical snake_case (decision D-7);
//! - model metadata and chat chunks keep the exact camelCase shape the Tauri
//!   frontend already consumes, so the desktop adapter can forward a runtime
//!   chunk to the window without redefining the UI payload.
//!
//! `api_key` travels in the request body over the loopback bearer connection; it
//! is never placed in a URL or a log line.

use crate::error::ErrorDetail;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Schema version carried by every [`ChatStreamEnvelope`].
pub const CHAT_STREAM_SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Model listing
// ---------------------------------------------------------------------------

/// `POST /control/v1/models/list` request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ListModelsRequest {
    /// API protocol, e.g. `openai`.
    pub api_protocol: String,
    /// Optional API base URL override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    /// Optional API key override. Body-only; never a URL or log value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Optional provider/model metadata (proxy settings, custom params, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

/// Chat protocol discriminator on the wire.
///
/// The variant names match `ChatProtocol`'s derived serde representation so a
/// [`ModelDetailsDto`] is byte-compatible with the canonical model descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatProtocolDto {
    OpenAI,
    Claude,
    Gemini,
    Ollama,
    HuggingFace,
    Decision,
}

/// Transport-neutral mirror of the canonical `ModelDetails` wire shape.
///
/// Field names and optionality match the canonical descriptor exactly; the
/// `contracts` crate cannot depend on the desktop domain type, so the desktop
/// adapter converts this DTO back into it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDetailsDto {
    pub id: String,
    pub name: String,
    pub protocol: ChatProtocolDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    pub reasoning: Option<bool>,
    pub function_call: Option<bool>,
    pub image_input: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

// ---------------------------------------------------------------------------
// Chat start / stop
// ---------------------------------------------------------------------------

/// `POST /control/v1/chats/{chat_id}/start` request body.
///
/// `chat_id` is repeated in the body so the request is self-describing; the
/// handler rejects a body that disagrees with the route slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChatStartRequest {
    pub provider_id: i64,
    pub model: String,
    pub chat_id: String,
    pub messages: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

/// `POST /control/v1/chats/{chat_id}/start` response body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChatStartResponse {
    pub chat_id: String,
    /// `true` once the runtime accepted the turn onto its chat dispatcher.
    pub accepted: bool,
}

/// `POST /control/v1/chats/{chat_id}/stop` request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChatStopRequest {
    pub chat_id: String,
    /// Optional API protocol; the runtime falls back to its only chat protocol.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_protocol: Option<String>,
}

/// `POST /control/v1/chats/{chat_id}/stop` response body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChatStopResponse {
    pub chat_id: String,
    pub stopped: bool,
}

// ---------------------------------------------------------------------------
// Chat SSE
// ---------------------------------------------------------------------------

/// Message kind of a streamed chat chunk.
///
/// Mirrors the canonical `MessageType` serde representation so a forwarded
/// chunk keeps the exact `type` token the frontend already switches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageTypeDto {
    Error,
    Finished,
    Reasoning,
    Reference,
    Text,
    Think,
    ToolCalls,
    ToolResults,
    Step,
}

/// Finish reason of a streamed chat chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FinishReasonDto {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Complete,
    Error,
}

/// Transport-neutral mirror of the canonical `ChatResponse` wire shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatResponseDto {
    pub chat_id: String,
    pub chunk: String,
    pub r#type: MessageTypeDto,
    pub metadata: Option<Value>,
    pub finish_reason: Option<FinishReasonDto>,
}

/// One typed SSE event of a chat stream.
///
/// The envelope can express every observable outcome of a turn: a streamed
/// chunk, a normal completion, a runtime error and a cancellation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatStreamEvent {
    /// A chunk to forward to the UI verbatim.
    Response { response: ChatResponseDto },
    /// The turn ended normally.
    Finished {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        finish_reason: Option<FinishReasonDto>,
    },
    /// The stream ended because of a runtime error.
    Error { error: ErrorDetail },
    /// The turn was cancelled through the stop route.
    Cancelled,
}

/// Versioned SSE envelope for one chat event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChatStreamEnvelope {
    pub schema_version: u32,
    pub chat_id: String,
    pub sequence: u64,
    pub event: ChatStreamEvent,
}

impl ChatStreamEnvelope {
    /// Builds an envelope with the current schema version.
    pub fn new(chat_id: impl Into<String>, sequence: u64, event: ChatStreamEvent) -> Self {
        Self {
            schema_version: CHAT_STREAM_SCHEMA_VERSION,
            chat_id: chat_id.into(),
            sequence,
            event,
        }
    }

    /// Whether this event terminates the stream.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.event,
            ChatStreamEvent::Finished { .. }
                | ChatStreamEvent::Error { .. }
                | ChatStreamEvent::Cancelled
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn list_models_request_is_snake_case_and_omits_absent_optionals() {
        let request = ListModelsRequest {
            api_protocol: "openai".to_string(),
            api_url: None,
            api_key: Some("secret".to_string()),
            metadata: None,
        };
        assert_eq!(
            serde_json::to_value(&request).expect("serialize"),
            json!({"api_protocol": "openai", "api_key": "secret"})
        );
    }

    #[test]
    fn model_details_matches_the_canonical_camel_case_shape() {
        let model = ModelDetailsDto {
            id: "gpt-4".to_string(),
            name: "GPT-4".to_string(),
            protocol: ChatProtocolDto::OpenAI,
            max_input_tokens: Some(128_000),
            max_output_tokens: None,
            description: None,
            last_updated: None,
            family: None,
            reasoning: None,
            function_call: Some(true),
            image_input: None,
            recommended_temperature: None,
            metadata: None,
        };
        assert_eq!(
            serde_json::to_value(&model).expect("serialize"),
            json!({
                "id": "gpt-4",
                "name": "GPT-4",
                "protocol": "OpenAI",
                "maxInputTokens": 128000,
                "reasoning": null,
                "functionCall": true,
                "imageInput": null,
            })
        );
    }

    #[test]
    fn chat_start_request_is_snake_case_and_round_trips() {
        let request = ChatStartRequest {
            provider_id: 7,
            model: "gpt-4".to_string(),
            chat_id: "chat-1".to_string(),
            messages: vec![json!({"role": "user", "content": "hi"})],
            network_enabled: Some(false),
            mcp_enabled: None,
            metadata: Some(json!({"windowLabel": "main"})),
        };
        let value = serde_json::to_value(&request).expect("serialize");
        assert_eq!(value["provider_id"], json!(7));
        assert_eq!(value["network_enabled"], json!(false));
        assert!(value.get("mcp_enabled").is_none());
        let parsed: ChatStartRequest = serde_json::from_value(value).expect("deserialize");
        assert_eq!(parsed, request);
    }

    #[test]
    fn chat_stop_request_keeps_the_route_slot_id() {
        let value = serde_json::to_value(ChatStopRequest {
            chat_id: "chat-1".to_string(),
            api_protocol: None,
        })
        .expect("serialize");
        assert_eq!(value, json!({"chat_id": "chat-1"}));
    }

    #[test]
    fn chat_response_matches_the_canonical_chunk_shape() {
        let response = ChatResponseDto {
            chat_id: "chat-1".to_string(),
            chunk: "hello".to_string(),
            r#type: MessageTypeDto::Text,
            metadata: Some(json!({"windowLabel": "main"})),
            finish_reason: None,
        };
        assert_eq!(
            serde_json::to_value(&response).expect("serialize"),
            json!({
                "chatId": "chat-1",
                "chunk": "hello",
                "type": "text",
                "metadata": {"windowLabel": "main"},
                "finishReason": null,
            })
        );
    }

    #[test]
    fn stream_envelope_tags_every_outcome() {
        let response = ChatStreamEnvelope::new(
            "chat-1",
            0,
            ChatStreamEvent::Response {
                response: ChatResponseDto {
                    chat_id: "chat-1".to_string(),
                    chunk: "hi".to_string(),
                    r#type: MessageTypeDto::Reasoning,
                    metadata: None,
                    finish_reason: None,
                },
            },
        );
        assert_eq!(
            serde_json::to_value(&response).expect("serialize"),
            json!({
                "schema_version": 1,
                "chat_id": "chat-1",
                "sequence": 0,
                "event": {"kind": "response", "response": {
                    "chatId": "chat-1",
                    "chunk": "hi",
                    "type": "reasoning",
                    "metadata": null,
                    "finishReason": null,
                }},
            })
        );
        assert!(!response.is_terminal());

        let finished = ChatStreamEnvelope::new(
            "chat-1",
            1,
            ChatStreamEvent::Finished {
                finish_reason: Some(FinishReasonDto::Stop),
            },
        );
        assert_eq!(
            serde_json::to_value(&finished).expect("serialize")["event"],
            json!({"kind": "finished", "finish_reason": "stop"})
        );
        assert!(finished.is_terminal());

        let cancelled = ChatStreamEnvelope::new("chat-1", 2, ChatStreamEvent::Cancelled);
        assert_eq!(
            serde_json::to_value(&cancelled).expect("serialize")["event"],
            json!({"kind": "cancelled"})
        );
        assert!(cancelled.is_terminal());

        let error = ChatStreamEnvelope::new(
            "chat-1",
            3,
            ChatStreamEvent::Error {
                error: ErrorDetail {
                    code: "chat_error".to_string(),
                    message: "boom".to_string(),
                },
            },
        );
        assert_eq!(
            serde_json::to_value(&error).expect("serialize")["event"],
            json!({"kind": "error", "error": {"code": "chat_error", "message": "boom"}})
        );
        assert!(error.is_terminal());
    }
}
