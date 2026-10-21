//! Desktop adapters for the runtime-owned chat and model surface.
//!
//! The standalone runtime is the single owner of model execution: it holds the
//! chat state, the tool registry and the provider proxy, and its own sessions
//! resolve every model call against that owner. The desktop `list_models`,
//! `chat_completion` and `stop_chat` commands therefore never run a local
//! executor, inject `MainStore`/`ChatState`, or open the runtime database: they
//! are thin HTTP adapters over the shared [`RuntimeSupervisor`] client.
//!
//! The protocol surface they use is the typed chat/model one:
//! - `POST /control/v1/models/list` for model listing;
//! - `POST /control/v1/chats/{chat_id}/start` for a turn;
//! - `GET /control/v1/chats/{chat_id}/events` for its typed SSE envelopes;
//! - `POST /control/v1/chats/{chat_id}/stop` for cancellation.
//!
//! `chat_completion` keeps its Tauri-only `window` boundary: it forwards each
//! runtime chat chunk onto the window's existing `chat_stream` event, so the
//! frontend wire is unchanged. The turn itself keeps running on the runtime when
//! the desktop task ends, and a disconnect never kills the runtime executor.

use crate::ai::traits::chat::{ChatResponse, FinishReason, MessageType, ModelDetails};
use crate::runtime_client::{client_error_wire, runtime_error_wire, RuntimeSupervisor};
use chatspeed_contracts::{
    ChatResponseDto, ChatStartRequest, ChatStopRequest, ChatStreamEvent, ListModelsRequest,
    ModelDetailsDto, UserErrorPresentation,
};
use chatspeed_runtime_backend::error_diagnostics::{ErrorProjection, FailureMessage};
use chatspeed_runtime_client::{ChatEventStream, ClientError, RuntimeClient};
use rust_i18n::t;
use serde_json::{json, Value};
use tauri::Emitter;

/// Lists the models a provider exposes, resolved by the runtime owner.
pub(crate) async fn list_models(
    supervisor: &RuntimeSupervisor,
    api_protocol: String,
    api_url: Option<&str>,
    api_key: Option<&str>,
    metadata: Option<Value>,
) -> Result<Vec<ModelDetails>, String> {
    let client = connected_client(supervisor).await?;
    let request = ListModelsRequest {
        api_protocol,
        api_url: api_url.map(str::to_string),
        api_key: api_key.map(str::to_string),
        metadata,
    };
    let models = client.list_models(&request).await.map_err(client_message)?;
    models.into_iter().map(model_details).collect()
}

/// Starts one chat turn on the runtime and forwards its stream to `window`.
///
/// The SSE subscription is established before the turn is started, so a chunk
/// produced immediately by a fast provider cannot be missed. The command returns
/// as soon as the runtime accepted the turn; the forwarding task owns the stream
/// afterwards.
pub(crate) async fn chat_completion(
    window: tauri::Window,
    supervisor: &RuntimeSupervisor,
    provider_id: i64,
    model: String,
    chat_id: String,
    messages: Vec<Value>,
    network_enabled: Option<bool>,
    mcp_enabled: Option<bool>,
    metadata: Option<Value>,
) -> Result<(), String> {
    let client = connected_client(supervisor).await?;

    let stream = client.stream_chat(&chat_id).await.map_err(client_message)?;

    let request = ChatStartRequest {
        provider_id,
        model,
        chat_id: chat_id.clone(),
        messages,
        network_enabled,
        mcp_enabled,
        metadata: Some(with_window_label(metadata, window.label())),
    };
    let started = client
        .start_chat(&chat_id, &request)
        .await
        .map_err(client_message)?;
    if !started.accepted {
        return Err("The runtime did not accept the chat turn".to_string());
    }

    tokio::spawn(forward_chat_events(
        window,
        chat_id,
        request.model.clone(),
        stream,
    ));
    Ok(())
}

/// Stops the runtime-owned chat for one chat id.
pub(crate) async fn stop_chat(
    supervisor: &RuntimeSupervisor,
    api_protocol: String,
    chat_id: &str,
) -> Result<(), String> {
    let client = connected_client(supervisor).await?;
    let request = ChatStopRequest {
        chat_id: chat_id.to_string(),
        api_protocol: Some(api_protocol),
    };
    client
        .stop_chat(chat_id, &request)
        .await
        .map_err(client_message)?;
    Ok(())
}

/// Resolves the connected control-plane client or the redacted reason there is
/// none, so callers can tell "runtime down" from a protocol error.
async fn connected_client(supervisor: &RuntimeSupervisor) -> Result<RuntimeClient, String> {
    supervisor.client().await.map_err(runtime_error_wire)
}

/// Serializes a public runtime rejection for the Tauri string boundary.
///
/// The command signature keeps `Err(String)`, so a structured `Server`/`Auth`
/// rejection is encoded as the shared error envelope: `code` and `message` stay
/// the legacy pair old readers consume, and the optional `presentation` keeps
/// the reason/upstream status the runtime attached. The frontend parser already
/// accepts this JSON string. Transport, discovery, protocol and serialization
/// failures stay plain technical strings because they carry no public envelope.
fn client_message(error: ClientError) -> String {
    client_error_wire(error)
}

/// Ensures the request metadata carries the window label the dispatcher routes by.
///
/// The runtime routes UI-bound chunks by the message's window label, so it must
/// be the window the frontend listens on. A label the client already set is kept
/// verbatim, matching the previous desktop behavior.
fn with_window_label(metadata: Option<Value>, window_label: &str) -> Value {
    let mut metadata = metadata.unwrap_or_else(|| json!({}));
    match metadata.as_object_mut() {
        Some(object) => {
            if !object.contains_key("windowLabel") && !object.contains_key("label") {
                object.insert("windowLabel".to_string(), json!(window_label));
            }
        }
        None => metadata = json!({ "windowLabel": window_label }),
    }
    metadata
}

/// Forwards the runtime's typed SSE events onto the window's `chat_stream` event.
///
/// The runtime publishes the canonical terminal chunk (a `finished` or `error`
/// response) before its typed terminal envelope, so a normal turn is forwarded
/// verbatim and the typed envelope is a no-op. A typed error that arrived without
/// its canonical chunk, a clean EOF and a read error would otherwise leave the UI
/// waiting forever, so the tail reports exactly one synthesized error chunk for
/// them. The runtime ends the SSE stream a client fell behind on, so the lagged
/// case reaches the desktop as that same EOF.
async fn forward_chat_events(
    window: tauri::Window,
    chat_id: String,
    model: String,
    mut stream: ChatEventStream,
) {
    let mut tail = ChatStreamTail::new(chat_id, window.label().to_string(), model);
    loop {
        match stream.next_event().await {
            Ok(Some(envelope)) => match envelope.event {
                ChatStreamEvent::Response { response } => match to_chat_response(response) {
                    Some(response) => {
                        let terminal = tail.forward_chunk(&response);
                        emit_chat_response(&window, response);
                        if terminal {
                            break;
                        }
                    }
                    None => log::error!(
                        "Failed to map a runtime chat chunk for window '{}'",
                        window.label()
                    ),
                },
                event => {
                    if let Some(response) = tail.terminal_response(&event) {
                        emit_chat_response(&window, response);
                    }
                    break;
                }
            },
            Ok(None) => {
                if let Some(response) = tail.detached_response(None) {
                    emit_chat_response(&window, response);
                }
                break;
            }
            Err(error) => {
                log::warn!(
                    "Chat stream for window '{}' ended: {}",
                    window.label(),
                    error
                );
                if let Some(response) = tail.detached_response(Some(&error.to_string())) {
                    emit_chat_response(&window, response);
                }
                break;
            }
        }
    }
}

/// Emits one canonical chat chunk onto the window's `chat_stream` event.
fn emit_chat_response(window: &tauri::Window, response: ChatResponse) {
    if let Err(error) = window.emit("chat_stream", response) {
        log::error!(
            "Failed to emit chat_stream for window '{}': {}",
            window.label(),
            error
        );
    }
}

/// The tail state of one forwarded chat stream.
///
/// It records whether the UI already received the turn's canonical terminal chunk
/// so a normal finish is never reported twice, and turns every other end of the
/// stream into exactly one canonical error chunk. It performs no I/O, so the
/// forwarder's terminal contract is verifiable without a window.
struct ChatStreamTail {
    chat_id: String,
    window_label: String,
    model: String,
    ended: bool,
}

impl ChatStreamTail {
    fn new(chat_id: String, window_label: String, model: String) -> Self {
        Self {
            chat_id,
            window_label,
            model,
            ended: false,
        }
    }

    /// Records one forwarded chunk; returns `true` when it was the terminal one.
    fn forward_chunk(&mut self, response: &ChatResponse) -> bool {
        let terminal = matches!(response.r#type, MessageType::Finished | MessageType::Error);
        if terminal {
            self.ended = true;
        }
        terminal
    }

    /// Completes typed-only terminal envelopes using the canonical UI contract.
    /// A prior terminal response wins, preventing duplicate completion.
    fn terminal_response(&mut self, event: &ChatStreamEvent) -> Option<ChatResponse> {
        if self.ended {
            return None;
        }
        self.ended = true;
        match event {
            ChatStreamEvent::Error { error } => Some(
                self.error_response(
                    &stream_error_message(&self.model, &error.message),
                    error
                        .presentation
                        .clone()
                        .unwrap_or_else(|| leaf_user_error(&error.message)),
                ),
            ),
            ChatStreamEvent::Finished { finish_reason } => Some(ChatResponse {
                chat_id: self.chat_id.clone(),
                chunk: String::new(),
                r#type: MessageType::Finished,
                metadata: Some(json!({ "windowLabel": self.window_label.as_str() })),
                finish_reason: (*finish_reason).map(|reason| match reason {
                    chatspeed_contracts::FinishReasonDto::Stop => FinishReason::Stop,
                    chatspeed_contracts::FinishReasonDto::Length => FinishReason::Length,
                    chatspeed_contracts::FinishReasonDto::ToolCalls => FinishReason::ToolCalls,
                    chatspeed_contracts::FinishReasonDto::ContentFilter => {
                        FinishReason::ContentFilter
                    }
                    chatspeed_contracts::FinishReasonDto::Complete => FinishReason::Complete,
                    chatspeed_contracts::FinishReasonDto::Error => FinishReason::Error,
                }),
            }),
            ChatStreamEvent::Cancelled => Some(ChatResponse {
                chat_id: self.chat_id.clone(),
                chunk: String::new(),
                r#type: MessageType::Finished,
                metadata: Some(json!({ "windowLabel": self.window_label.as_str() })),
                finish_reason: Some(FinishReason::Stop),
            }),
            ChatStreamEvent::Response { .. } => None,
        }
    }

    /// Reports a stream that ended without a typed envelope.
    ///
    /// A clean EOF (`error` is `None`) and a read error both end the UI, unless
    /// the turn already ended normally.
    fn detached_response(&mut self, error: Option<&str>) -> Option<ChatResponse> {
        if self.ended {
            return None;
        }
        self.ended = true;
        let cause = match error {
            Some(error) => error.to_string(),
            None => t!("tools.error.network_error").to_string(),
        };
        Some(self.error_response(
            &stream_error_message(&self.model, &cause),
            leaf_user_error(&cause),
        ))
    }

    /// Builds the canonical error chunk that ends the UI for this turn.
    fn error_response(&self, message: &str, user_error: UserErrorPresentation) -> ChatResponse {
        ChatResponse {
            chat_id: self.chat_id.clone(),
            chunk: json!({ "status": 500, "message": message }).to_string(),
            r#type: MessageType::Error,
            metadata: Some(json!({
                "windowLabel": self.window_label.as_str(),
                "user_error": user_error
            })),
            finish_reason: Some(FinishReason::Error),
        }
    }
}

/// Projects a synthesized stream-end cause without wrapping it in the display
/// template, so the safe user text is the concrete cause itself.
fn leaf_user_error(cause: &str) -> UserErrorPresentation {
    FailureMessage::leaf(cause).user_error()
}

/// Renders the existing chat stream error key for a synthesized stream end.
///
/// `provider` carries the model that was streaming, because the desktop adapter
/// receives a provider id and a model rather than a provider display name, and
/// `details` carries the concrete cause.
fn stream_error_message(model: &str, cause: &str) -> String {
    t!(
        "chat.error.stream_processing_failed",
        provider = model,
        details = cause
    )
    .to_string()
}

/// Converts a runtime chat DTO back into the canonical chunk the UI consumes.
///
/// The DTO mirrors the canonical wire shape exactly, so this is a lossless
/// serde round trip and the emitted payload stays byte-identical to the previous
/// in-process behavior.
fn to_chat_response(response: ChatResponseDto) -> Option<ChatResponse> {
    let value = serde_json::to_value(&response).ok()?;
    serde_json::from_value(value).ok()
}

fn model_details(model: ModelDetailsDto) -> Result<ModelDetails, String> {
    let value = serde_json::to_value(&model).map_err(|error| error.to_string())?;
    serde_json::from_value(value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chatspeed_contracts::{
        ErrorDetail, ErrorReason, FinishReasonDto, MessageTypeDto, UserErrorPresentation,
    };
    use chatspeed_runtime_client::{AuthError, ClientResponseError};

    #[tokio::test]
    async fn a_disconnected_runtime_reports_availability_instead_of_running_locally() {
        let supervisor = RuntimeSupervisor::new();

        let error = list_models(&supervisor, "openai".to_string(), None, None, None)
            .await
            .expect_err("a disconnected runtime must not answer");
        assert!(error.contains("not connected"), "error was {error}");

        let error = stop_chat(&supervisor, "openai".to_string(), "chat-1")
            .await
            .expect_err("a disconnected runtime must not stop anything");
        assert!(error.contains("not connected"), "error was {error}");
    }

    #[test]
    fn a_list_models_rejection_keeps_its_presentation_on_the_ipc_wire() {
        let presentation = UserErrorPresentation {
            schema_version: 1,
            message: "This provider does not expose a model list".to_string(),
            reason: Some(ErrorReason::UnsupportedListModels),
            upstream_status: Some(405),
        };
        let wire = client_message(ClientError::Server(ClientResponseError {
            legacy_display: "server error 405: method not allowed (method_not_allowed)".to_string(),
            status: 405,
            code: "method_not_allowed".to_string(),
            message: "provider rejected the model list request".to_string(),
            presentation: Some(presentation),
        }));

        let value: Value = serde_json::from_str(&wire).expect("a JSON error envelope");
        assert_eq!(value["status"], 405);
        assert_eq!(value["error"]["code"], "method_not_allowed");
        assert_eq!(
            value["error"]["message"],
            "provider rejected the model list request"
        );
        assert_eq!(
            value["error"]["presentation"]["reason"],
            "unsupported_list_models"
        );
        assert_eq!(value["error"]["presentation"]["upstream_status"], 405);
        // Only the public envelope crosses; no raw headers or body fields exist.
        assert!(value["error"].get("headers").is_none());
        assert!(value["error"].get("body").is_none());
    }

    #[test]
    fn an_auth_rejection_preserves_its_status_code_and_category() {
        let wire = client_message(ClientError::Auth(AuthError {
            status: 401,
            code: "unauthorized".to_string(),
            message: "invalid runtime token".to_string(),
            presentation: None,
        }));

        let value: Value = serde_json::from_str(&wire).expect("a JSON error envelope");
        assert_eq!(value["status"], 401);
        assert_eq!(value["error"]["code"], "unauthorized");
        assert_eq!(value["error"]["message"], "invalid runtime token");
        assert!(value["error"].get("presentation").is_none());
    }

    #[test]
    fn a_technical_client_failure_stays_a_plain_string() {
        let wire = client_message(ClientError::Transport("connection reset".to_string()));
        assert_eq!(wire, "transport error: connection reset");
        assert!(!wire.starts_with('{'));
    }

    #[test]
    fn the_window_label_is_filled_only_when_absent() {
        let filled = with_window_label(None, "main");
        assert_eq!(filled["windowLabel"], json!("main"));

        let kept = with_window_label(Some(json!({ "windowLabel": "other" })), "main");
        assert_eq!(kept["windowLabel"], json!("other"));

        let labelled = with_window_label(Some(json!({ "label": "alt" })), "main");
        assert_eq!(labelled["label"], json!("alt"));
        assert!(labelled.get("windowLabel").is_none());
    }

    #[test]
    fn a_runtime_chunk_maps_back_to_the_canonical_window_payload() {
        let dto = ChatResponseDto {
            chat_id: "chat-1".to_string(),
            chunk: "hi".to_string(),
            r#type: MessageTypeDto::Text,
            metadata: Some(json!({ "windowLabel": "main" })),
            finish_reason: None,
        };
        let response = to_chat_response(dto.clone()).expect("map the chunk");
        assert_eq!(
            serde_json::to_value(&response).expect("response json"),
            serde_json::to_value(&dto).expect("dto json")
        );
    }

    #[test]
    fn a_runtime_model_descriptor_maps_back_to_the_canonical_type() {
        let dto = ModelDetailsDto {
            id: "gpt-4".to_string(),
            name: "GPT-4".to_string(),
            protocol: chatspeed_contracts::ChatProtocolDto::OpenAI,
            max_input_tokens: Some(128_000),
            max_output_tokens: None,
            description: None,
            last_updated: None,
            family: None,
            reasoning: Some(true),
            function_call: None,
            image_input: None,
            recommended_temperature: None,
            metadata: None,
        };
        let details = model_details(dto.clone()).expect("map the model");
        assert_eq!(
            serde_json::to_value(&details).expect("details json"),
            serde_json::to_value(&dto).expect("dto json")
        );
    }

    fn chunk(
        chat_id: &str,
        text: &str,
        kind: MessageType,
        reason: Option<FinishReason>,
    ) -> ChatResponse {
        ChatResponse {
            chat_id: chat_id.to_string(),
            chunk: text.to_string(),
            r#type: kind,
            metadata: Some(json!({ "windowLabel": "main" })),
            finish_reason: reason,
        }
    }

    /// Reads the canonical `{ status, message }` error chunk the UI parses.
    fn error_message(response: &ChatResponse) -> String {
        let parsed: Value = serde_json::from_str(&response.chunk).expect("canonical error chunk");
        assert_eq!(parsed["status"], json!(500));
        parsed["message"]
            .as_str()
            .expect("error chunk carries a message")
            .to_string()
    }

    fn typed_error(message: &str) -> ChatStreamEvent {
        ChatStreamEvent::Error {
            error: ErrorDetail {
                code: "chat_stream_detached".to_string(),
                message: message.to_string(),
                presentation: None,
            },
        }
    }

    #[test]
    fn a_normal_finish_forwards_its_terminal_chunk_without_a_second_one() {
        let mut tail = ChatStreamTail::new(
            "chat-1".to_string(),
            "main".to_string(),
            "gpt-4".to_string(),
        );

        assert!(!tail.forward_chunk(&chunk("chat-1", "think", MessageType::Reasoning, None)));
        assert!(tail.forward_chunk(&chunk(
            "chat-1",
            "done",
            MessageType::Finished,
            Some(FinishReason::Complete),
        )));

        // The runtime's typed terminal after the canonical chunk must not
        // synthesize a second terminal chunk.
        assert!(tail
            .terminal_response(&ChatStreamEvent::Finished {
                finish_reason: Some(FinishReasonDto::Complete),
            })
            .is_none());
        assert!(tail.detached_response(None).is_none());
    }

    #[test]
    fn a_typed_error_without_its_chunk_reaches_the_ui_exactly_once() {
        let mut tail = ChatStreamTail::new(
            "chat-1".to_string(),
            "main".to_string(),
            "gpt-4".to_string(),
        );
        assert!(!tail.forward_chunk(&chunk("chat-1", "think", MessageType::Reasoning, None)));

        let event = typed_error("The chat turn stopped producing output");
        let response = tail
            .terminal_response(&event)
            .expect("one synthesized error");
        assert_eq!(response.chat_id, "chat-1");
        assert_eq!(response.r#type, MessageType::Error);
        assert_eq!(response.finish_reason, Some(FinishReason::Error));
        let metadata = response.metadata.as_ref().expect("error metadata");
        assert_eq!(metadata["windowLabel"], "main");
        assert_eq!(
            metadata["user_error"]["message"],
            "The chat turn stopped producing output"
        );
        assert_eq!(metadata["user_error"]["schema_version"], 1);
        assert!(error_message(&response).contains("stopped producing output"));

        // A typed presentation from the runtime wins over the safe fallback.
        let mut presented = ChatStreamTail::new(
            "chat-1".to_string(),
            "main".to_string(),
            "gpt-4".to_string(),
        );
        let presented_event = ChatStreamEvent::Error {
            error: ErrorDetail {
                code: "chat_error".to_string(),
                message: "legacy transport text".to_string(),
                presentation: Some(UserErrorPresentation {
                    schema_version: 1,
                    message: "Provider declined the request".to_string(),
                    reason: Some(chatspeed_contracts::ErrorReason::UnsupportedListModels),
                    upstream_status: Some(405),
                }),
            },
        };
        let presented_response = presented
            .terminal_response(&presented_event)
            .expect("one synthesized error");
        let presented_metadata = presented_response.metadata.as_ref().expect("metadata");
        assert_eq!(
            presented_metadata["user_error"]["message"],
            "Provider declined the request"
        );
        assert_eq!(presented_metadata["user_error"]["upstream_status"], 405);
        assert!(error_message(&presented_response).contains("legacy transport text"));

        // Duplicate protection: no later end may add another terminal chunk.
        assert!(tail.terminal_response(&event).is_none());
        assert!(tail.detached_response(None).is_none());
    }

    #[test]
    fn an_eof_or_read_error_ends_the_ui_with_one_error_chunk() {
        let mut eof = ChatStreamTail::new(
            "chat-1".to_string(),
            "main".to_string(),
            "gpt-4".to_string(),
        );
        let eof_response = eof.detached_response(None).expect("one synthesized error");
        assert_eq!(eof_response.chat_id, "chat-1");
        assert_eq!(eof_response.r#type, MessageType::Error);
        assert_eq!(eof_response.finish_reason, Some(FinishReason::Error));
        let eof_metadata = eof_response.metadata.as_ref().expect("error metadata");
        assert_eq!(eof_metadata["windowLabel"], "main");
        assert!(eof_metadata["user_error"]["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty()));
        assert!(!error_message(&eof_response).is_empty());
        assert!(eof.detached_response(None).is_none());

        let mut read = ChatStreamTail::new(
            "chat-2".to_string(),
            "main".to_string(),
            "gpt-4".to_string(),
        );
        let read_response = read
            .detached_response(Some("connection reset"))
            .expect("one synthesized error");
        assert_eq!(read_response.chat_id, "chat-2");
        assert!(error_message(&read_response).contains("connection reset"));
        assert_eq!(
            read_response.metadata.as_ref().expect("metadata")["user_error"]["message"],
            "connection reset"
        );
    }

    #[test]
    fn typed_only_completion_and_cancellation_end_the_ui_once() {
        for event in [
            ChatStreamEvent::Finished {
                finish_reason: Some(FinishReasonDto::Complete),
            },
            ChatStreamEvent::Cancelled,
        ] {
            let mut tail = ChatStreamTail::new(
                "chat-1".to_string(),
                "main".to_string(),
                "gpt-4".to_string(),
            );
            assert!(!tail.forward_chunk(&chunk("chat-1", "hi", MessageType::Text, None)));
            let response = tail.terminal_response(&event).expect("terminal UI chunk");
            assert_eq!(response.r#type, MessageType::Finished);
            assert_eq!(response.chat_id, "chat-1");
            assert_eq!(response.metadata, Some(json!({ "windowLabel": "main" })));
            assert!(tail.terminal_response(&event).is_none());
            assert!(tail.detached_response(None).is_none());
        }
    }
}
