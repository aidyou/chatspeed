//! Runtime-owned chat and model routes on the canonical control plane.
//!
//! The standalone runtime is the single owner of model listing, chat execution
//! and chat cancellation. These routes expose that owner through the same
//! bearer-protected, typed `/control/v1` server the workflow, capability and
//! automation routes use, so the desktop client (and `cs` later) has no reason to
//! run a second executor or open the runtime database:
//!
//! - `POST /control/v1/models/list` calls the canonical `list_models_async`
//!   against the owner's `MainStore`;
//! - `POST /control/v1/chats/{chat_id}/start` calls the canonical
//!   `start_new_chat_interaction` against the owner's `ChatState`, so tool
//!   execution and the chat dispatcher stay the one authoritative path;
//! - `GET /control/v1/chats/{chat_id}/events` relays the dispatcher's UI-bound
//!   chunks as typed SSE envelopes;
//! - `POST /control/v1/chats/{chat_id}/stop` sets the canonical stop flag and
//!   signals the stream that the turn was cancelled.
//!
//! # Delivery path
//!
//! `start_new_chat_interaction` keeps the canonical dispatcher callback (passing
//! a custom one would bypass tool-call execution), so its output reaches a client
//! through the shared window-channel registry the dispatcher already writes to.
//! This module registers one channel per turn under the request's window label and
//! forwards every dispatcher chunk into a per-chat [`ChatStreamBroker`] the SSE
//! route subscribes to. The broker is chat-scoped observation only: it is not a
//! second workflow authority and holds no execution state.
//!
//! # Lifecycle
//!
//! - the events route creates the per-chat broker entry and terminates the stream
//!   on the first terminal envelope (finished/error/cancelled);
//! - the forwarder closes its window channel when the turn ends, so the
//!   dispatcher can never write into an abandoned channel;
//! - the stop route closes the same channel, so a cancelled turn ends promptly
//!   even if its task never emits a terminal chunk;
//! - a broker entry is removed as soon as it has no subscribers, so the map stays
//!   bounded by the chats a client is actually observing.
//!
//! The runtime owns no WebView: this module is compiled only for the desktop-free
//! build and the desktop control plane keeps exactly its previous routes.

use crate::ai::interaction::chat_completion::{
    list_models_async, start_new_chat_interaction, ChatState,
};
use crate::ai::traits::chat::{ChatMetadata, ChatResponse, MessageType, ModelDetails};
use crate::ccproxy::ChatProtocol;
use crate::db::MainStore;
use crate::libs::window_channels::ChannelHandle;
use crate::workflow::react::client::http::dto;
use crate::workflow::react::client::http::server::{with_idempotency, ControlPlaneState};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chatspeed_contracts::{
    ChatProtocolDto, ChatResponseDto, ChatStartRequest, ChatStartResponse, ChatStopRequest,
    ChatStopResponse, ChatStreamEnvelope, ChatStreamEvent, ErrorDetail, FinishReasonDto,
    ListModelsRequest, MessageTypeDto, ModelDetailsDto,
};
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::convert::Infallible;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;

/// `POST /control/v1/models/list` route template.
pub const MODELS_LIST_PATH: &str = "/control/v1/models/list";
/// `POST /control/v1/chats/{chat_id}/start` route template.
pub const CHAT_START_PATH: &str = "/control/v1/chats/{chat_id}/start";
/// `POST /control/v1/chats/{chat_id}/stop` route template.
pub const CHAT_STOP_PATH: &str = "/control/v1/chats/{chat_id}/stop";
/// `GET /control/v1/chats/{chat_id}/events` route template.
pub const CHAT_EVENTS_PATH: &str = "/control/v1/chats/{chat_id}/events";

/// Bounded capacity of a per-chat SSE broadcast.
const STREAM_CAPACITY: usize = 256;
/// Bounded capacity of the SSE response pump.
const PUMP_CAPACITY: usize = 64;

/// The runtime owner's canonical chat handles.
///
/// Exposing the two handles (rather than a second execution API) keeps the
/// handlers on the exact canonical functions the runtime already runs, while the
/// desktop control plane never links this trait.
pub trait RuntimeChatPlane: Send + Sync + 'static {
    /// The runtime's single database authority.
    fn main_store(&self) -> Arc<MainStore>;
    /// The runtime's chat/tool execution state.
    fn chat_state(&self) -> Arc<ChatState>;
}

/// Builds the runtime chat/model router extension.
pub fn chat_router() -> Router<ControlPlaneState> {
    Router::new()
        .route(MODELS_LIST_PATH, post(list_runtime_models))
        .route(CHAT_START_PATH, post(start_runtime_chat))
        .route(CHAT_STOP_PATH, post(stop_runtime_chat))
        .route(CHAT_EVENTS_PATH, get(stream_runtime_chat))
}

// ---------------------------------------------------------------------------
// Per-chat SSE broker
// ---------------------------------------------------------------------------

/// Per-chat broadcast of typed chat envelopes.
///
/// An entry is created when a client subscribes or starts a turn and removed once
/// it has no subscribers, so a disconnected client leaves nothing behind.
pub struct ChatStreamBroker {
    inner: Mutex<HashMap<String, Arc<ChatStreamEntry>>>,
}

impl ChatStreamBroker {
    /// Creates an empty broker.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Returns the entry for `chat_id`, creating it when absent.
    pub fn entry(&self, chat_id: &str) -> Arc<ChatStreamEntry> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner
            .entry(chat_id.to_string())
            .or_insert_with(|| Arc::new(ChatStreamEntry::new(chat_id)))
            .clone()
    }

    /// Returns the entry for `chat_id` if one exists.
    pub fn get(&self, chat_id: &str) -> Option<Arc<ChatStreamEntry>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(chat_id)
            .cloned()
    }

    /// Publishes one event to every subscriber of `chat_id`, if any.
    pub fn publish(&self, chat_id: &str, event: ChatStreamEvent) {
        if let Some(entry) = self.get(chat_id) {
            entry.publish(event);
        }
    }

    /// Removes the entry once nothing observes it any more.
    pub fn remove_if_idle(&self, chat_id: &str) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if inner.get(chat_id).is_some_and(|entry| entry.is_idle()) {
            inner.remove(chat_id);
        }
    }
}

impl Default for ChatStreamBroker {
    fn default() -> Self {
        Self::new()
    }
}

/// One chat's broadcast sender, sequence counter and window-channel handle.
pub struct ChatStreamEntry {
    chat_id: String,
    sender: broadcast::Sender<ChatStreamEnvelope>,
    sequence: AtomicU64,
    channel: tokio::sync::Mutex<Option<ChannelHandle>>,
}

impl ChatStreamEntry {
    fn new(chat_id: &str) -> Self {
        let (sender, _receiver) = broadcast::channel(STREAM_CAPACITY);
        Self {
            chat_id: chat_id.to_string(),
            sender,
            sequence: AtomicU64::new(0),
            channel: tokio::sync::Mutex::new(None),
        }
    }

    /// Subscribes one SSE observer.
    pub fn subscribe(&self) -> broadcast::Receiver<ChatStreamEnvelope> {
        self.sender.subscribe()
    }

    /// Publishes one envelope with the next per-chat sequence number.
    pub fn publish(&self, event: ChatStreamEvent) {
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);
        let envelope = ChatStreamEnvelope::new(self.chat_id.clone(), sequence, event);
        let _ = self.sender.send(envelope);
    }

    fn is_idle(&self) -> bool {
        self.sender.receiver_count() == 0
    }

    /// Attaches the window channel the turn's dispatcher routes into.
    pub async fn attach_channel(&self, handle: ChannelHandle) {
        *self.channel.lock().await = Some(handle);
    }

    /// Unregisters the attached channel, ending the turn's delivery path.
    pub async fn close_channel(&self) {
        let handle = self.channel.lock().await.take();
        if let Some(handle) = handle {
            handle.close().await;
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /control/v1/models/list`.
async fn list_runtime_models(State(state): State<ControlPlaneState>, body: String) -> Response {
    let plane = match require_chat_plane(&state) {
        Ok(plane) => plane,
        Err(response) => return response,
    };
    let request: ListModelsRequest = match parse_body(&body, "model list request") {
        Ok(request) => request,
        Err(response) => return response,
    };

    match list_models_async(
        plane.main_store(),
        request.api_protocol,
        request.api_url.as_deref(),
        request.api_key.as_deref(),
        request.metadata,
    )
    .await
    {
        Ok(models) => {
            let models = models
                .into_iter()
                .map(ModelDetailsDto::from)
                .collect::<Vec<_>>();
            Json(models).into_response()
        }
        Err(error) => dto::error_response(
            StatusCode::BAD_GATEWAY,
            "model_list_failed",
            error.to_string(),
        ),
    }
}

/// `POST /control/v1/chats/{chat_id}/start`.
async fn start_runtime_chat(
    State(state): State<ControlPlaneState>,
    Path(chat_id): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    with_idempotency(
        &state,
        &headers,
        &body,
        move |state: ControlPlaneState, body: String| async move {
            start_runtime_chat_inner(state, chat_id, body).await
        },
    )
    .await
}

async fn start_runtime_chat_inner(
    state: ControlPlaneState,
    chat_id: String,
    body: String,
) -> Response {
    let plane = match require_chat_plane(&state) {
        Ok(plane) => plane,
        Err(response) => return response,
    };
    let request: ChatStartRequest = match parse_body(&body, "chat start request") {
        Ok(request) => request,
        Err(response) => return response,
    };
    if request.chat_id != chat_id {
        return invalid_input("The chat start body must carry the route's chat id".to_string());
    }
    if request.provider_id < 1 {
        return invalid_input("The chat start request must carry a provider id".to_string());
    }
    if request.model.trim().is_empty() {
        return invalid_input("The chat start request must carry a model".to_string());
    }
    if request.messages.is_empty() {
        return invalid_input("The chat start request must carry messages".to_string());
    }

    let mut metadata = match request.metadata {
        Some(value) => match serde_json::from_value::<ChatMetadata>(value) {
            Ok(metadata) => metadata,
            Err(error) => {
                return invalid_input(format!("Invalid chat metadata: {error}"));
            }
        },
        None => ChatMetadata::default(),
    };
    // The dispatcher routes UI-bound chunks by the message's window label. The
    // runtime forces it to the client's label (falling back to the chat id) and
    // registers exactly that channel, so the label is also the delivery key. The
    // label is preserved on every forwarded chunk, so the frontend keeps seeing
    // its own window label.
    let window_label = metadata
        .window_label
        .clone()
        .or_else(|| metadata.label.clone())
        .unwrap_or_else(|| chat_id.clone());
    metadata.window_label = Some(window_label.clone());

    let chat_state = plane.chat_state();
    let registration = chat_state.channels.register_channel(&window_label).await;
    let entry = state.chat_streams.entry(&chat_id);
    entry.attach_channel(registration.handle).await;

    let broker = state.chat_streams.clone();
    let forward_chat_id = chat_id.clone();
    let mut receiver = registration.receiver;
    tokio::spawn(async move {
        forward_chat_responses(broker, forward_chat_id, &mut receiver).await;
    });

    match start_new_chat_interaction(
        chat_state,
        request.provider_id,
        request.model,
        chat_id.clone(),
        request.messages,
        None,
        Some(metadata),
        None,
    )
    .await
    {
        Ok(()) => Json(ChatStartResponse {
            chat_id,
            accepted: true,
        })
        .into_response(),
        Err(error) => {
            // The turn never reached the dispatcher: terminate any observer the
            // client already attached with an explicit typed error.
            state.chat_streams.publish(
                &chat_id,
                ChatStreamEvent::Error {
                    error: ErrorDetail {
                        code: "chat_start_failed".to_string(),
                        message: error.to_string(),
                    },
                },
            );
            end_chat_stream(&state.chat_streams, &chat_id).await;
            dto::error_response(
                StatusCode::BAD_REQUEST,
                "chat_start_failed",
                error.to_string(),
            )
        }
    }
}

/// `POST /control/v1/chats/{chat_id}/stop`.
async fn stop_runtime_chat(
    State(state): State<ControlPlaneState>,
    Path(chat_id): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    with_idempotency(
        &state,
        &headers,
        &body,
        move |state: ControlPlaneState, body: String| async move {
            stop_runtime_chat_inner(state, chat_id, body).await
        },
    )
    .await
}

async fn stop_runtime_chat_inner(
    state: ControlPlaneState,
    chat_id: String,
    body: String,
) -> Response {
    let plane = match require_chat_plane(&state) {
        Ok(plane) => plane,
        Err(response) => return response,
    };
    let request: ChatStopRequest = match parse_body(&body, "chat stop request") {
        Ok(request) => request,
        Err(response) => return response,
    };
    if request.chat_id != chat_id {
        return invalid_input("The chat stop body must carry the route's chat id".to_string());
    }

    // The runtime registers every chat under the OpenAI protocol, so an absent or
    // unrecognized protocol still resolves to the chat the client started.
    let protocol = request
        .api_protocol
        .as_deref()
        .and_then(|value| ChatProtocol::from_str(value).ok())
        .unwrap_or(ChatProtocol::OpenAI);

    let chat_state = plane.chat_state();
    let stopped = {
        let mut chats = chat_state.chats.lock().await;
        let mut stopped = false;
        if let Some(by_chat) = chats.get_mut(&protocol) {
            if let Some(chat) = by_chat.get_mut(&chat_id) {
                chat.set_stop_flag(true).await;
                by_chat.remove(&chat_id);
                stopped = true;
            }
        }
        stopped
    };

    state
        .chat_streams
        .publish(&chat_id, ChatStreamEvent::Cancelled);
    end_chat_stream(&state.chat_streams, &chat_id).await;

    Json(ChatStopResponse { chat_id, stopped }).into_response()
}

/// `GET /control/v1/chats/{chat_id}/events`.
async fn stream_runtime_chat(
    State(state): State<ControlPlaneState>,
    Path(chat_id): Path<String>,
) -> Response {
    if state.chat.is_none() {
        return runtime_unavailable_response();
    }

    let entry = state.chat_streams.entry(&chat_id);
    let mut subscription = entry.subscribe();
    let broker = state.chat_streams.clone();
    let stream_chat_id = chat_id.clone();
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(PUMP_CAPACITY);

    tokio::spawn(async move {
        loop {
            match subscription.recv().await {
                Ok(envelope) => {
                    let terminal = envelope.is_terminal();
                    if tx.send(Ok(envelope_event(&envelope))).await.is_err() {
                        break;
                    }
                    if terminal {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => break,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
        broker.remove_if_idle(&stream_chat_id);
    });

    Sse::new(ReceiverStream::new(rx))
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Forwards the dispatcher's UI-bound chunks into the per-chat broker.
///
/// The dispatcher already suppressed mid-turn `tool_calls` finish markers, so any
/// forwarded `finished` or `error` chunk is the end of the turn; the forwarder then
/// closes the window channel and leaves the broker entry to the last observer.
/// A turn whose channel is closed before a terminal chunk (for example because a
/// newer turn took over the same window label) still publishes a terminal error,
/// so its observers never wait on a stream that can no longer produce output.
async fn forward_chat_responses(
    broker: Arc<ChatStreamBroker>,
    chat_id: String,
    receiver: &mut mpsc::Receiver<Arc<ChatResponse>>,
) {
    let mut finished = false;
    while let Some(chunk) = receiver.recv().await {
        let response = ChatResponseDto::from(chunk.as_ref());
        let terminal = matches!(
            response.r#type,
            MessageTypeDto::Finished | MessageTypeDto::Error
        );
        let finish_reason = response.finish_reason;
        let is_error = matches!(response.r#type, MessageTypeDto::Error);
        broker.publish(&chat_id, ChatStreamEvent::Response { response });
        if terminal {
            if is_error {
                broker.publish(
                    &chat_id,
                    ChatStreamEvent::Error {
                        error: ErrorDetail {
                            code: "chat_error".to_string(),
                            message: "The chat turn ended with an error".to_string(),
                        },
                    },
                );
            } else {
                broker.publish(&chat_id, ChatStreamEvent::Finished { finish_reason });
            }
            finished = true;
            break;
        }
    }
    if !finished {
        broker.publish(
            &chat_id,
            ChatStreamEvent::Error {
                error: ErrorDetail {
                    code: "chat_stream_detached".to_string(),
                    message: "The chat turn stopped producing output".to_string(),
                },
            },
        );
    }
    end_chat_stream(&broker, &chat_id).await;
}

/// Closes the turn's window channel and drops an unobserved broker entry.
async fn end_chat_stream(broker: &ChatStreamBroker, chat_id: &str) {
    if let Some(entry) = broker.get(chat_id) {
        entry.close_channel().await;
    }
    broker.remove_if_idle(chat_id);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn require_chat_plane(state: &ControlPlaneState) -> Result<Arc<dyn RuntimeChatPlane>, Response> {
    state.chat.clone().ok_or_else(runtime_unavailable_response)
}

/// Stable answer for a control plane whose process owns no runtime chat.
fn runtime_unavailable_response() -> Response {
    dto::error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "runtime_unavailable",
        "This control plane does not own the runtime chat executor".to_string(),
    )
}

fn invalid_input(message: String) -> Response {
    dto::error_response(StatusCode::BAD_REQUEST, "invalid_input", message)
}

fn parse_body<T: DeserializeOwned>(body: &str, what: &str) -> Result<T, Response> {
    serde_json::from_str(body).map_err(|error| invalid_input(format!("Invalid {what}: {error}")))
}

fn envelope_event(envelope: &ChatStreamEnvelope) -> Event {
    let data = serde_json::to_string(envelope).unwrap_or_else(|_| "{}".to_string());
    Event::default()
        .id(envelope.sequence.to_string())
        .data(data)
}

impl From<ModelDetails> for ModelDetailsDto {
    fn from(model: ModelDetails) -> Self {
        Self {
            id: model.id,
            name: model.name,
            protocol: model.protocol.into(),
            max_input_tokens: model.max_input_tokens,
            max_output_tokens: model.max_output_tokens,
            description: model.description,
            last_updated: model.last_updated,
            family: model.family,
            reasoning: model.reasoning,
            function_call: model.function_call,
            image_input: model.image_input,
            recommended_temperature: model.recommended_temperature,
            metadata: model.metadata,
        }
    }
}

impl From<ChatProtocol> for ChatProtocolDto {
    fn from(protocol: ChatProtocol) -> Self {
        match protocol {
            ChatProtocol::OpenAI => Self::OpenAI,
            ChatProtocol::Claude => Self::Claude,
            ChatProtocol::Gemini => Self::Gemini,
            ChatProtocol::Ollama => Self::Ollama,
            ChatProtocol::HuggingFace => Self::HuggingFace,
            ChatProtocol::Decision => Self::Decision,
        }
    }
}

impl From<&ChatResponse> for ChatResponseDto {
    fn from(response: &ChatResponse) -> Self {
        Self {
            chat_id: response.chat_id.clone(),
            chunk: response.chunk.clone(),
            r#type: message_type(&response.r#type),
            metadata: response.metadata.clone(),
            finish_reason: response.finish_reason.as_ref().map(finish_reason),
        }
    }
}

fn message_type(message_type: &MessageType) -> MessageTypeDto {
    match message_type {
        MessageType::Error => MessageTypeDto::Error,
        MessageType::Finished => MessageTypeDto::Finished,
        MessageType::Reasoning => MessageTypeDto::Reasoning,
        MessageType::Reference => MessageTypeDto::Reference,
        MessageType::Text => MessageTypeDto::Text,
        MessageType::Think => MessageTypeDto::Think,
        MessageType::ToolCalls => MessageTypeDto::ToolCalls,
        MessageType::ToolResults => MessageTypeDto::ToolResults,
        MessageType::Step => MessageTypeDto::Step,
    }
}

fn finish_reason(reason: &crate::ai::traits::chat::FinishReason) -> FinishReasonDto {
    use crate::ai::traits::chat::FinishReason;
    match reason {
        FinishReason::Stop => FinishReasonDto::Stop,
        FinishReason::Length => FinishReasonDto::Length,
        FinishReason::ToolCalls => FinishReasonDto::ToolCalls,
        FinishReason::ContentFilter => FinishReasonDto::ContentFilter,
        FinishReason::Complete => FinishReasonDto::Complete,
        FinishReason::Error => FinishReasonDto::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::traits::chat::FinishReason;
    use chatspeed_contracts::ChatProtocolDto;
    use serde_json::json;

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
            metadata: Some(json!({"windowLabel": "main"})),
            finish_reason: reason,
        }
    }

    #[test]
    fn the_route_templates_match_the_documented_control_plane_paths() {
        assert_eq!(MODELS_LIST_PATH, "/control/v1/models/list");
        assert_eq!(CHAT_START_PATH, "/control/v1/chats/{chat_id}/start");
        assert_eq!(CHAT_STOP_PATH, "/control/v1/chats/{chat_id}/stop");
        assert_eq!(CHAT_EVENTS_PATH, "/control/v1/chats/{chat_id}/events");
    }

    #[test]
    fn the_model_dto_matches_the_canonical_descriptor_wire_shape() {
        let model = ModelDetails {
            id: "gpt-4".to_string(),
            name: "GPT-4".to_string(),
            protocol: ChatProtocol::OpenAI,
            max_input_tokens: Some(128_000),
            max_output_tokens: None,
            description: None,
            last_updated: None,
            family: None,
            reasoning: Some(true),
            function_call: Some(true),
            image_input: None,
            recommended_temperature: None,
            metadata: None,
        };
        assert_eq!(
            serde_json::to_value(ModelDetailsDto::from(model.clone())).expect("dto json"),
            serde_json::to_value(&model).expect("model json")
        );
    }

    #[test]
    fn every_protocol_maps_to_its_wire_variant() {
        for (protocol, expected) in [
            (ChatProtocol::OpenAI, ChatProtocolDto::OpenAI),
            (ChatProtocol::Claude, ChatProtocolDto::Claude),
            (ChatProtocol::Gemini, ChatProtocolDto::Gemini),
            (ChatProtocol::Ollama, ChatProtocolDto::Ollama),
            (ChatProtocol::HuggingFace, ChatProtocolDto::HuggingFace),
            (ChatProtocol::Decision, ChatProtocolDto::Decision),
        ] {
            assert_eq!(ChatProtocolDto::from(protocol), expected);
        }
    }

    #[test]
    fn the_chat_dto_matches_the_canonical_chunk_wire_shape() {
        let response = chunk(
            "chat-1",
            "hello",
            MessageType::Finished,
            Some(FinishReason::Stop),
        );
        assert_eq!(
            serde_json::to_value(ChatResponseDto::from(&response)).expect("dto json"),
            serde_json::to_value(&response).expect("response json")
        );
    }

    #[tokio::test]
    async fn the_broker_assigns_monotonic_sequences_and_stops_on_terminal_events() {
        let broker = ChatStreamBroker::new();
        let entry = broker.entry("chat-1");
        let mut subscription = entry.subscribe();

        entry.publish(ChatStreamEvent::Response {
            response: ChatResponseDto::from(&chunk("chat-1", "hi", MessageType::Text, None)),
        });
        entry.publish(ChatStreamEvent::Finished {
            finish_reason: Some(FinishReasonDto::Stop),
        });

        let first = subscription.recv().await.expect("first envelope");
        assert_eq!(first.sequence, 0);
        assert_eq!(first.chat_id, "chat-1");
        assert!(!first.is_terminal());
        let second = subscription.recv().await.expect("second envelope");
        assert_eq!(second.sequence, 1);
        assert!(second.is_terminal());

        drop(subscription);
        broker.remove_if_idle("chat-1");
        assert!(broker.get("chat-1").is_none());
    }

    #[tokio::test]
    async fn publishing_without_an_observer_is_a_no_op() {
        let broker = ChatStreamBroker::new();
        broker.publish("chat-1", ChatStreamEvent::Cancelled);
        assert!(broker.get("chat-1").is_none());
    }

    #[tokio::test]
    async fn the_forwarder_publishes_chunks_then_a_terminal_envelope() {
        let broker = Arc::new(ChatStreamBroker::new());
        let entry = broker.entry("chat-1");
        let mut subscription = entry.subscribe();
        let (tx, mut rx) = mpsc::channel::<Arc<ChatResponse>>(8);
        tx.send(Arc::new(chunk(
            "chat-1",
            "think",
            MessageType::Reasoning,
            None,
        )))
        .await
        .expect("reasoning");
        tx.send(Arc::new(chunk(
            "chat-1",
            "done",
            MessageType::Finished,
            Some(FinishReason::Complete),
        )))
        .await
        .expect("finished");
        drop(tx);

        forward_chat_responses(broker.clone(), "chat-1".to_string(), &mut rx).await;

        let reasoning = subscription.recv().await.expect("reasoning envelope");
        assert_eq!(reasoning.sequence, 0);
        assert_eq!(
            reasoning.event,
            ChatStreamEvent::Response {
                response: ChatResponseDto::from(&chunk(
                    "chat-1",
                    "think",
                    MessageType::Reasoning,
                    None
                )),
            }
        );

        // The terminal chunk is forwarded verbatim first, so the frontend keeps
        // receiving the exact canonical payload, then the stream is closed.
        let finished_chunk = subscription.recv().await.expect("finished chunk");
        assert_eq!(finished_chunk.sequence, 1);
        assert_eq!(
            finished_chunk.event,
            ChatStreamEvent::Response {
                response: ChatResponseDto::from(&chunk(
                    "chat-1",
                    "done",
                    MessageType::Finished,
                    Some(FinishReason::Complete),
                )),
            }
        );
        let terminal = subscription.recv().await.expect("terminal envelope");
        assert_eq!(terminal.sequence, 2);
        assert_eq!(
            terminal.event,
            ChatStreamEvent::Finished {
                finish_reason: Some(FinishReasonDto::Complete),
            }
        );
        assert!(terminal.is_terminal());
    }

    #[tokio::test]
    async fn an_error_chunk_terminates_the_stream_with_a_typed_error() {
        let broker = Arc::new(ChatStreamBroker::new());
        let entry = broker.entry("chat-1");
        let mut subscription = entry.subscribe();
        let (tx, mut rx) = mpsc::channel::<Arc<ChatResponse>>(8);
        tx.send(Arc::new(chunk(
            "chat-1",
            "{\"error\":\"boom\"}",
            MessageType::Error,
            Some(FinishReason::Error),
        )))
        .await
        .expect("error chunk");
        drop(tx);

        forward_chat_responses(broker.clone(), "chat-1".to_string(), &mut rx).await;

        let response = subscription.recv().await.expect("error response");
        assert!(matches!(response.event, ChatStreamEvent::Response { .. }));
        let terminal = subscription.recv().await.expect("terminal error");
        assert!(matches!(terminal.event, ChatStreamEvent::Error { .. }));
        assert!(terminal.is_terminal());
    }

    #[tokio::test]
    async fn a_channel_closed_without_a_terminal_chunk_still_ends_the_stream() {
        let broker = Arc::new(ChatStreamBroker::new());
        let entry = broker.entry("chat-1");
        let mut subscription = entry.subscribe();
        // A closed channel with no terminal chunk models a turn whose window
        // label was taken over by a newer turn.
        let (_tx, mut rx) = mpsc::channel::<Arc<ChatResponse>>(8);
        drop(_tx);

        forward_chat_responses(broker.clone(), "chat-1".to_string(), &mut rx).await;

        let terminal = subscription.recv().await.expect("terminal error");
        assert_eq!(
            terminal.event,
            ChatStreamEvent::Error {
                error: ErrorDetail {
                    code: "chat_stream_detached".to_string(),
                    message: "The chat turn stopped producing output".to_string(),
                },
            }
        );
        assert!(terminal.is_terminal());
    }
}
