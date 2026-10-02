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

use crate::ai::traits::chat::{ChatResponse, ModelDetails};
use crate::runtime_client::RuntimeSupervisor;
use chatspeed_contracts::{
    ChatResponseDto, ChatStartRequest, ChatStopRequest, ChatStreamEvent, ListModelsRequest,
    ModelDetailsDto,
};
use chatspeed_runtime_client::{ChatEventStream, ClientError, RuntimeClient};
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

    tokio::spawn(forward_chat_events(window, stream));
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
    supervisor.client().await.map_err(|error| error.to_string())
}

fn client_message(error: ClientError) -> String {
    error.to_string()
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
async fn forward_chat_events(window: tauri::Window, mut stream: ChatEventStream) {
    loop {
        match stream.next_event().await {
            Ok(Some(envelope)) => {
                let ChatStreamEvent::Response { response } = envelope.event else {
                    // Finished / error / cancelled closes the turn.
                    break;
                };
                match to_chat_response(response) {
                    Some(chat_response) => {
                        if let Err(error) = window.emit("chat_stream", chat_response) {
                            log::error!(
                                "Failed to emit chat_stream for window '{}': {}",
                                window.label(),
                                error
                            );
                        }
                    }
                    None => log::error!(
                        "Failed to map a runtime chat chunk for window '{}'",
                        window.label()
                    ),
                }
            }
            Ok(None) => break,
            Err(error) => {
                log::warn!(
                    "Chat stream for window '{}' ended: {}",
                    window.label(),
                    error
                );
                break;
            }
        }
    }
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
    use chatspeed_contracts::MessageTypeDto;

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
}
