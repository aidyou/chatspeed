//! This module defines the routing for the `ccproxy` server. It handles dispatching
//! incoming HTTP requests for various AI models (OpenAI, Claude, Gemini, Ollama)
//! to the appropriate backend logic.
//!
//! # !!!!!!!!!!!!!!!!!!!!! CRITICAL WARNING !!!!!!!!!!!!!!!!!!!!!
//! **DO NOT modify the routing structure or change the existing route paths.**
//! The hierarchical routing design (Fixed Prefixes -> Direct Routes -> Grouped Routes)
//! is carefully ordered to prevent route shadowing and maintain client compatibility.
//! Any changes to the router composition in `routes()` must be thoroughly tested.
//!
//! # Core Concepts
//!
//! ### Tool Compatibility Mode
//! This mode allows models that don't natively support tool-calling to gain this capability.
//! When a request is sent to a `compat_mode` endpoint, `ccproxy` instructs the language model
//! to generate formatted tool calls, which are then parsed and returned to the client in a
//! standardized format.
//!
//! To use this mode, prefix the standard API path with `/compat_mode` (or `/compat`).
//!
//! ### Grouped Model Access & Dynamic Switching
//! `ccproxy` allows organizing models into groups (e.g., `qwen`).
//! To access a group, prefix the path with the group name, like `/qwen/v1/...`.
//!
//! The special prefix `/switch` (e.g., `/switch/v1/...`) automatically routes to the
//! currently "Active" group set in the application settings, allowing for dynamic switching
//! without changing client configurations.
//!
//! # API Endpoints
//!
//! The following sections detail available API endpoints. Tool Compatibility Mode,
//! Grouped Model Access, and Dynamic Switching can be combined where applicable.
//!
//! ### General Endpoints
//! - `GET /`: Returns a welcome message from the ccproxy.
//! - `GET /api/version`: Shows the current `ccproxy` version.
//!
//! ### OpenAI-Compatible Endpoints
//! These endpoints mimic the OpenAI API.
//! - `GET /v1/models`: Lists available models.
//! - `POST /v1/chat/completions`: Creates a chat completion.
//! - `POST /v1/responses`: Creates an OpenAI Responses-compatible non-streaming response.
//! - `POST /v1/embeddings`: Creates an embedding vector.
//!
//! ### Claude-Compatible Endpoints
//! - `POST /v1/messages`: Creates a message with the Claude model.
//! - `POST /v1/claude/embeddings`: Creates an embedding vector.
//!   *(Note: Claude natively does not support embeddings; this endpoint is provided for protocol consistency).*
//!
//! ### Gemini-Compatible Endpoints
//! - `GET /v1beta/models`: Lists available Gemini models.
//! - `POST /v1beta/models/{model_id}:{action}`: Standard Gemini format.
//! - `POST /v1beta/models/{model_id}/{action}`: Support for slash-style action path.
//!
//! ### Ollama-Specific Endpoints
//! - `GET /api/tags`: Lists local Ollama models.
//! - `POST /api/show`: Returns information about a specific Ollama model.
//! - `POST /api/chat`: Creates a chat completion with an Ollama model.
//! - `POST /api/embed` or `/api/embeddings`: Creates embedding vectors.
//!
//! ### System One Decision Endpoint
//! - `POST /v1/systemone`: Evaluates one state against typed questions through a decision model.
//!   Decision requests are never a chat protocol, so no compatibility mode applies to this path.
//!
//! ### Integrated Module Endpoints (Non-ccproxy core)
//! These routes are integrated into this router for unified access but handled by separate modules:
//! - **MCP (Model Context Protocol)**:
//!   - `POST /mcp/http`: Streamable HTTP entry (Recommended).
//!
//! # Internal Architecture
//!
//! 1.  **Logic Handlers**:
//!     -   Functions like `openai_chat_logic` and `gemini_list_models_logic` act as the entry
//!         bridge between Axum routes and backend proxy handlers.
//!     -   They utilize `resolve_group_name` to process dynamic prefixes, ensuring `/switch`
//!         correctly resolves to the currently active group.
//!
//! 2.  **Protocol-Specific Route Builders**:
//!     -   `openai_routes`, `gemini_routes`, etc., generate sub-routers for various
//!         access modes (Direct, Grouped, and Compat).
//!
//! 3.  **Middleware Layer**:
//!     -   `authenticate_request_middleware` enforces security across all endpoints,
//!         with a loopback bypass for local Ollama usage.
//!
//! 4.  **Hierarchical Router Composition**:
//!     -   The `routes` function assembles the final `Router` using a strict priority order:
//!         **Fixed Prefixes** (MCP/HTTP) > **Direct Routes** (/v1) > **Global Compat** (/compat) > **Grouped Routes** (/{group}).
//!     -   This ordering prevents route shadowing, ensuring that static protocol paths are
//!         not misidentified as dynamic group names.
//!

use crate::ai::interaction::chat_completion::ChatState;
use crate::ccproxy::errors::CCProxyError;
use crate::ccproxy::ChatProtocol;
use crate::ccproxy::{
    auth::{authenticate_request, is_trusted_internal_request},
    handle_chat_completion, handle_decision, handle_embedding, handle_list_models,
    handle_ollama_tags, handle_responses,
    handler::{handle_gemini_list_models, handle_ollama_show, ollama_extra_handler::ShowRequest},
    helper::CcproxyQuery,
};
use crate::constants::CFG_ACTIVE_PROXY_GROUP;
use crate::db::MainStore;

use axum::{
    body::HttpBody as _,
    extract::{ConnectInfo, Path, Query, State},
    http::{self, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use bytes::Bytes;
use futures_util::StreamExt as _;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;

const TOOL_COMPAT_MODE_PREFIX: &str = "compat_mode";
const TOOL_COMPAT_SHORT_PREFIX: &str = "compat";
const SWITCH_MODE_PREFIX: &str = "switch";

// A struct to hold the shared state, which is passed to all route handlers.
pub struct SharedState {
    /// The application package version reported by `/api/version`. It is an
    /// explicit value rather than a `tauri::AppHandle`, so the same router can
    /// be mounted by a windowless (`chatspeed-headless`) process without
    /// fabricating a desktop handle (INV-1/INV-4).
    pub package_version: String,
    pub main_store: Arc<MainStore>,
    pub chat_state: Arc<ChatState>,
}

// ----------------------------------------------------------------------------
// Core Logic Handlers
// ----------------------------------------------------------------------------
// These handlers contain the actual business logic. They are called by the
// axum handlers below and are agnostic of the routing layer.

/// Resolves the group name for a request. If the prefix is 'switch', it reads the active group from settings.
fn resolve_group_name(state: &Arc<SharedState>, group_name: Option<String>) -> Option<String> {
    if let Some(g) = group_name {
        if g == SWITCH_MODE_PREFIX {
            Some(
                state
                    .main_store
                    .get_config(CFG_ACTIVE_PROXY_GROUP, "default".to_string()),
            )
        } else {
            Some(g)
        }
    } else {
        None
    }
}

/// Stable `runtime_stopping` refusal in the ccproxy error envelope.
///
/// It carries the same `runtime_stopping` code the control plane returns, so a
/// proxy client can tell a deliberately closed admission gate apart from a
/// provider failure and re-read `/control/v1/runtime/status` instead of
/// retrying against an instance that is shutting down.
fn runtime_stopping_response(denied: &crate::lifecycle::AdmissionDenied) -> Response {
    let body = json!({
        "error": {
            "code": chatspeed_contracts::RUNTIME_CODE_RUNTIME_STOPPING,
            "message": denied.message,
            "type": "Runtime Stopping",
        }
    });
    (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
}

/// Keeps one admission held until the response body has been fully delivered.
///
/// A buffered body carries an exact length hint that the framework uses to frame
/// the response, so it is left byte-for-byte untouched and the admission ends
/// with the handler: a buffered response has already finished its provider work.
/// A streaming body keeps producing bytes after the handler returned, so the
/// admission is moved into the byte stream and released only once the stream
/// ends or the client disconnects, which is what lets the shutdown drain observe
/// an in-flight stream.
fn hold_admission(response: Response, ticket: crate::lifecycle::AdmissionTicket) -> Response {
    let (parts, body) = response.into_parts();
    if body.size_hint().exact().is_some() {
        drop(ticket);
        return Response::from_parts(parts, body);
    }
    let stream = body.into_data_stream().map(move |chunk| {
        let _ = &ticket;
        chunk
    });
    Response::from_parts(parts, axum::body::Body::from_stream(stream))
}

/// Admits one unit of proxy provider work on the owner's gate, then returns the
/// handler's response with the admission held until its body ends.
///
/// This is the same gate the control plane and the canonical chat entry use, so
/// a quiescing runtime refuses proxy chat and embedding work with the stable
/// `runtime_stopping` code. Read-only routes (model listing, version) never call
/// it, so they stay readable.
async fn gated<F>(
    state: &SharedState,
    kind: crate::lifecycle::WorkKind,
    work: F,
) -> Response
where
    F: std::future::Future<Output = Result<Response, CCProxyError>>,
{
    let ticket = match state.chat_state.admit_work(kind) {
        Ok(ticket) => ticket,
        Err(denied) => return runtime_stopping_response(&denied),
    };
    let response = work.await.into_response();
    hold_admission(response, ticket)
}

async fn openai_chat_logic(
    state: Arc<SharedState>,
    query: CcproxyQuery,
    headers: HeaderMap,
    body: Bytes,
    group_name: Option<String>,
    compat_mode: bool,
) -> Result<Response, CCProxyError> {
    let final_group = resolve_group_name(&state, group_name);

    Ok(gated(
        &state,
        crate::lifecycle::WorkKind::ChatTurn,
        handle_chat_completion(
            ChatProtocol::OpenAI,
            headers,
            query,
            body,
            final_group,
            compat_mode,
            "".to_string(), // model_id
            "".to_string(), // action
            state.main_store.clone(),
        ),
    )
    .await)
}

async fn openai_responses_logic(
    state: Arc<SharedState>,
    query: CcproxyQuery,
    headers: HeaderMap,
    body: Bytes,
    group_name: Option<String>,
    compat_mode: bool,
) -> Result<Response, CCProxyError> {
    let final_group = resolve_group_name(&state, group_name);

    Ok(gated(
        &state,
        crate::lifecycle::WorkKind::ChatTurn,
        handle_responses(
            headers,
            query,
            body,
            final_group,
            compat_mode,
            state.main_store.clone(),
        ),
    )
    .await)
}

async fn claude_chat_logic(
    state: Arc<SharedState>,
    query: CcproxyQuery,
    headers: HeaderMap,
    body: Bytes,
    group_name: Option<String>,
    compat_mode: bool,
) -> Result<Response, CCProxyError> {
    let final_group = resolve_group_name(&state, group_name);

    Ok(gated(
        &state,
        crate::lifecycle::WorkKind::ChatTurn,
        handle_chat_completion(
            ChatProtocol::Claude,
            headers,
            query,
            body,
            final_group,
            compat_mode,
            "".to_string(), // model_id
            "".to_string(), // action
            state.main_store.clone(),
        ),
    )
    .await)
}

async fn gemini_chat_logic(
    state: Arc<SharedState>,
    query: CcproxyQuery,
    headers: HeaderMap,
    body: Bytes,
    group_name: Option<String>,
    compat_mode: bool,
    model_id: String,
    action: String,
) -> Result<Response, CCProxyError> {
    let final_group = resolve_group_name(&state, group_name);

    Ok(gated(
        &state,
        crate::lifecycle::WorkKind::ChatTurn,
        handle_chat_completion(
            ChatProtocol::Gemini,
            headers,
            query,
            body,
            final_group,
            compat_mode,
            model_id,
            action,
            state.main_store.clone(),
        ),
    )
    .await)
}

async fn ollama_chat_logic(
    state: Arc<SharedState>,
    query: CcproxyQuery,
    headers: HeaderMap,
    body: Bytes,
    group_name: Option<String>,
    compat_mode: bool,
) -> Result<Response, CCProxyError> {
    let final_group = resolve_group_name(&state, group_name);

    Ok(gated(
        &state,
        crate::lifecycle::WorkKind::ChatTurn,
        handle_chat_completion(
            ChatProtocol::Ollama,
            headers,
            query,
            body,
            final_group,
            compat_mode,
            "".to_string(), // model_id
            "".to_string(), // action
            state.main_store.clone(),
        ),
    )
    .await)
}

async fn openai_embedding_logic(
    state: Arc<SharedState>,
    query: CcproxyQuery,
    headers: HeaderMap,
    body: Bytes,
    group_name: Option<String>,
) -> Result<Response, CCProxyError> {
    let final_group = resolve_group_name(&state, group_name);

    Ok(gated(
        &state,
        crate::lifecycle::WorkKind::ChatTurn,
        handle_embedding(
            ChatProtocol::OpenAI,
            headers,
            query,
            body,
            final_group,
            "".to_string(),
            state.main_store.clone(),
        ),
    )
    .await)
}

async fn gemini_embedding_logic(
    state: Arc<SharedState>,
    query: CcproxyQuery,
    headers: HeaderMap,
    body: Bytes,
    group_name: Option<String>,
    model_id: String,
) -> Result<Response, CCProxyError> {
    let final_group = resolve_group_name(&state, group_name);

    Ok(gated(
        &state,
        crate::lifecycle::WorkKind::ChatTurn,
        handle_embedding(
            ChatProtocol::Gemini,
            headers,
            query,
            body,
            final_group,
            model_id,
            state.main_store.clone(),
        ),
    )
    .await)
}

async fn ollama_embedding_logic(
    state: Arc<SharedState>,
    query: CcproxyQuery,
    headers: HeaderMap,
    body: Bytes,
    group_name: Option<String>,
) -> Result<Response, CCProxyError> {
    let final_group = resolve_group_name(&state, group_name);

    Ok(gated(
        &state,
        crate::lifecycle::WorkKind::ChatTurn,
        handle_embedding(
            ChatProtocol::Ollama,
            headers,
            query,
            body,
            final_group,
            "".to_string(),
            state.main_store.clone(),
        ),
    )
    .await)
}

async fn claude_embedding_logic(
    state: Arc<SharedState>,
    query: CcproxyQuery,
    headers: HeaderMap,
    body: Bytes,
    group_name: Option<String>,
) -> Result<Response, CCProxyError> {
    let final_group = resolve_group_name(&state, group_name);

    Ok(gated(
        &state,
        crate::lifecycle::WorkKind::ChatTurn,
        handle_embedding(
            ChatProtocol::Claude,
            headers,
            query,
            body,
            final_group,
            "".to_string(),
            state.main_store.clone(),
        ),
    )
    .await)
}

async fn openai_list_models_logic(
    state: Arc<SharedState>,
    group_name: Option<String>,
) -> Result<Response, CCProxyError> {
    let final_group = resolve_group_name(&state, group_name);
    handle_list_models(final_group, state.main_store.clone())
        .await
        .map(|res| res.into_response())
}

async fn gemini_list_models_logic(
    state: Arc<SharedState>,
    group_name: Option<String>,
) -> Result<Response, CCProxyError> {
    let final_group = resolve_group_name(&state, group_name);
    handle_gemini_list_models(final_group, state.main_store.clone())
        .await
        .map(|res| res.into_response())
}

async fn ollama_list_tags_logic(
    state: Arc<SharedState>,
    group_name: Option<String>,
) -> Result<Response, CCProxyError> {
    let final_group = resolve_group_name(&state, group_name);
    handle_ollama_tags(final_group, state.main_store.clone())
        .await
        .map(|res| res.into_response())
}

async fn decision_logic(
    state: Arc<SharedState>,
    headers: HeaderMap,
    body: Bytes,
    group_name: Option<String>,
) -> Result<Response, CCProxyError> {
    let final_group = resolve_group_name(&state, group_name);
    let main_store = state.main_store.clone();

    // A System One decision evaluation is a real decision-model provider call, so
    // it is admission-gated exactly like the chat and embedding routes. Without
    // this the single provider entry `/v1/systemone` would keep starting provider
    // work after the gate closed, breaking the one admission decision that fences
    // every other canonical provider route.
    Ok(gated(
        &state,
        crate::lifecycle::WorkKind::ChatTurn,
        async move {
            handle_decision(headers, body, final_group, main_store)
                .await
                .map(|res| res.into_response())
        },
    )
    .await)
}

// ----------------------------------------------------------------------------
// Protocol-specific Route Builders
// ----------------------------------------------------------------------------

enum GroupMode {
    None,
    Path,
}

/// Creates routes for OpenAI-compatible endpoints.
fn openai_routes(compat_mode: bool, mode: GroupMode) -> Router<Arc<SharedState>> {
    match mode {
        GroupMode::Path => {
            let chat_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Path(group_name): Path<String>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    openai_chat_logic(state, query, headers, body, Some(group_name), compat_mode)
                        .await
                        .map_err(|e| e.into_response())
                },
            );
            let responses_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Path(group_name): Path<String>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    openai_responses_logic(
                        state,
                        query,
                        headers,
                        body,
                        Some(group_name),
                        compat_mode,
                    )
                    .await
                    .map_err(|e| e.into_response())
                },
            );
            let list_model_handler = get(
                |State(state): State<Arc<SharedState>>, Path(group_name): Path<String>| async move {
                    openai_list_models_logic(state, Some(group_name))
                        .await
                        .map_err(|e| e.into_response())
                },
            );
            let embedding_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Path(group_name): Path<String>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    openai_embedding_logic(state, query, headers, body, Some(group_name))
                        .await
                        .map_err(|e| e.into_response())
                },
            );
            Router::new()
                .route("/v1/chat/completions", chat_handler)
                .route("/v1/responses", responses_handler)
                .route("/v1/models", list_model_handler)
                .route("/v1/embeddings", embedding_handler)
        }
        GroupMode::None => {
            let chat_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    openai_chat_logic(state, query, headers, body, None, compat_mode)
                        .await
                        .map_err(|e| e.into_response())
                },
            );
            let responses_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    openai_responses_logic(state, query, headers, body, None, compat_mode)
                        .await
                        .map_err(|e| e.into_response())
                },
            );
            let list_model_handler = get(|State(state): State<Arc<SharedState>>| async move {
                openai_list_models_logic(state, None)
                    .await
                    .map_err(|e| e.into_response())
            });
            let embedding_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    openai_embedding_logic(state, query, headers, body, None)
                        .await
                        .map_err(|e| e.into_response())
                },
            );
            Router::new()
                .route("/v1/chat/completions", chat_handler)
                .route("/v1/responses", responses_handler)
                .route("/v1/models", list_model_handler)
                .route("/v1/embeddings", embedding_handler)
        }
    }
}

/// Creates routes for Claude-compatible endpoints.
fn claude_routes(compat_mode: bool, mode: GroupMode) -> Router<Arc<SharedState>> {
    match mode {
        GroupMode::Path => {
            let chat_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Path(group_name): Path<String>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    claude_chat_logic(state, query, headers, body, Some(group_name), compat_mode)
                        .await
                        .map_err(|e| e.into_response())
                },
            );
            let embedding_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Path(group_name): Path<String>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    claude_embedding_logic(state, query, headers, body, Some(group_name))
                        .await
                        .map_err(|e| e.into_response())
                },
            );
            Router::new()
                .route("/v1/messages", chat_handler)
                .route("/v1/claude/embeddings", embedding_handler)
        }
        GroupMode::None => {
            let chat_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    claude_chat_logic(state, query, headers, body, None, compat_mode)
                        .await
                        .map_err(|e| e.into_response())
                },
            );
            let embedding_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    claude_embedding_logic(state, query, headers, body, None)
                        .await
                        .map_err(|e| e.into_response())
                },
            );
            Router::new()
                .route("/v1/messages", chat_handler)
                .route("/v1/claude/embeddings", embedding_handler)
        }
    }
}

/// Creates routes for Gemini-compatible endpoints.
fn gemini_routes(compat_mode: bool, mode: GroupMode) -> Router<Arc<SharedState>> {
    match mode {
        GroupMode::Path => {
            let chat_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Path((group_name, model_id, action)): Path<(String, String, String)>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    if action == "embedContent" {
                        return gemini_embedding_logic(
                            state,
                            query,
                            headers,
                            body,
                            Some(group_name),
                            model_id,
                        )
                        .await
                        .map_err(|e| e.into_response());
                    }
                    gemini_chat_logic(
                        state,
                        query,
                        headers,
                        body,
                        Some(group_name),
                        compat_mode,
                        model_id,
                        action,
                    )
                    .await
                    .map_err(|e| e.into_response())
                },
            );
            let chat_handler_colon = post(
                move |State(state): State<Arc<SharedState>>,
                      Path((group_name, model_and_action)): Path<(String, String)>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    let (model_id, action) = model_and_action
                        .rsplit_once(':')
                        .ok_or_else(|| CCProxyError::InvalidProtocolError(model_and_action.clone()))
                        .map_err(|e| e.into_response())?;
                    if action == "embedContent" {
                        return gemini_embedding_logic(
                            state,
                            query,
                            headers,
                            body,
                            Some(group_name),
                            model_id.to_string(),
                        )
                        .await
                        .map_err(|e| e.into_response());
                    }
                    gemini_chat_logic(
                        state,
                        query,
                        headers,
                        body,
                        Some(group_name),
                        compat_mode,
                        model_id.to_string(),
                        action.to_string(),
                    )
                    .await
                    .map_err(|e| e.into_response())
                },
            );
            let list_model_handler = get(
                |State(state): State<Arc<SharedState>>, Path(group_name): Path<String>| async move {
                    gemini_list_models_logic(state, Some(group_name))
                        .await
                        .map_err(|e| e.into_response())
                },
            );
            Router::new()
                .route("/v1beta/models/{model_id}/{action}", chat_handler)
                .route("/v1beta/models/{model_and_action}", chat_handler_colon)
                .route("/v1beta/models", list_model_handler)
        }
        GroupMode::None => {
            let chat_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Path((model_id, action)): Path<(String, String)>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    if action == "embedContent" {
                        return gemini_embedding_logic(state, query, headers, body, None, model_id)
                            .await
                            .map_err(|e| e.into_response());
                    }
                    gemini_chat_logic(
                        state,
                        query,
                        headers,
                        body,
                        None,
                        compat_mode,
                        model_id,
                        action,
                    )
                    .await
                    .map_err(|e| e.into_response())
                },
            );
            let chat_handler_colon = post(
                move |State(state): State<Arc<SharedState>>,
                      Path(model_and_action): Path<String>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    let (model_id, action) = model_and_action
                        .rsplit_once(':')
                        .ok_or_else(|| CCProxyError::InvalidProtocolError(model_and_action.clone()))
                        .map_err(|e| e.into_response())?;
                    if action == "embedContent" {
                        return gemini_embedding_logic(
                            state,
                            query,
                            headers,
                            body,
                            None,
                            model_id.to_string(),
                        )
                        .await
                        .map_err(|e| e.into_response());
                    }
                    gemini_chat_logic(
                        state,
                        query,
                        headers,
                        body,
                        None,
                        compat_mode,
                        model_id.to_string(),
                        action.to_string(),
                    )
                    .await
                    .map_err(|e| e.into_response())
                },
            );
            let list_model_handler = get(|State(state): State<Arc<SharedState>>| async move {
                gemini_list_models_logic(state, None)
                    .await
                    .map_err(|e| e.into_response())
            });
            Router::new()
                .route("/v1beta/models/{model_id}/{action}", chat_handler)
                .route("/v1beta/models/{model_and_action}", chat_handler_colon)
                .route("/v1beta/models", list_model_handler)
        }
    }
}

/// Creates chat routes for Ollama-compatible endpoints.
fn ollama_chat_routes(compat_mode: bool, mode: GroupMode) -> Router<Arc<SharedState>> {
    match mode {
        GroupMode::Path => {
            let chat_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Path(group_name): Path<String>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    ollama_chat_logic(state, query, headers, body, Some(group_name), compat_mode)
                        .await
                        .map_err(|e| e.into_response())
                },
            );
            let list_model_handler = get(
                |State(state): State<Arc<SharedState>>, Path(group_name): Path<String>| async move {
                    ollama_list_tags_logic(state, Some(group_name))
                        .await
                        .map_err(|e| e.into_response())
                },
            );
            let embedding_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Path(group_name): Path<String>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    ollama_embedding_logic(state, query, headers, body, Some(group_name))
                        .await
                        .map_err(|e| e.into_response())
                },
            );
            Router::new()
                .route("/api/chat", chat_handler)
                .route("/api/tags", list_model_handler)
                .route("/api/embeddings", embedding_handler.clone())
                .route("/api/embed", embedding_handler)
        }
        GroupMode::None => {
            let chat_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    ollama_chat_logic(state, query, headers, body, None, compat_mode)
                        .await
                        .map_err(|e| e.into_response())
                },
            );
            let list_model_handler = get(|State(state): State<Arc<SharedState>>| async move {
                ollama_list_tags_logic(state, None)
                    .await
                    .map_err(|e| e.into_response())
            });
            let embedding_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    ollama_embedding_logic(state, query, headers, body, None)
                        .await
                        .map_err(|e| e.into_response())
                },
            );
            Router::new()
                .route("/api/chat", chat_handler)
                .route("/api/tags", list_model_handler)
                .route("/api/embeddings", embedding_handler.clone())
                .route("/api/embed", embedding_handler)
        }
    }
}

/// Creates non-chat API routes for Ollama (version, tags, etc.).
/// These routes do not support grouping or compatibility modes.
fn ollama_api_routes() -> Router<Arc<SharedState>> {
    Router::new()
        .route(
            "/api/version",
            get(|State(state): State<Arc<SharedState>>| async move {
                let version = state.package_version.clone();
                (StatusCode::OK, axum::Json(json!({ "version": version }))).into_response()
            }),
        )
        .route(
            "/api/show",
            post(
                |State(state): State<Arc<SharedState>>, body: Json<ShowRequest>| async move {
                    handle_ollama_show(state.main_store.clone(), body)
                        .await
                        .map_err(|e| e.into_response())
                },
            ),
        )
}

/// Creates routes for the System One decision endpoint.
///
/// Decision requests have no compatibility mode, so only direct and grouped access exist.
fn decision_routes(mode: GroupMode) -> Router<Arc<SharedState>> {
    match mode {
        GroupMode::Path => {
            let decision_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Path(group_name): Path<String>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    decision_logic(state, headers, body, Some(group_name))
                        .await
                        .map_err(|e| e.into_response())
                },
            );
            Router::new().route("/v1/systemone", decision_handler)
        }
        GroupMode::None => {
            let decision_handler = post(
                move |State(state): State<Arc<SharedState>>, headers: HeaderMap, body: Bytes| async move {
                    decision_logic(state, headers, body, None)
                        .await
                        .map_err(|e| e.into_response())
                },
            );
            Router::new().route("/v1/systemone", decision_handler)
        }
    }
}

// ----------------------------------------------------------------------------
// Main Router Definition
// ----------------------------------------------------------------------------

/// Defines all routes for the ccproxy module.
///
/// `package_version` replaces the former `tauri::AppHandle` dependency: it is
/// the only value the router ever read from the handle (`/api/version`), so the
/// router is now transport-neutral. Route composition, order, auth middleware,
/// model resolution and response header filtering are unchanged.
pub async fn routes(
    package_version: String,
    main_store_arc: Arc<MainStore>,
    chat_state: Arc<ChatState>,
) -> Router {
    let shared_state = Arc::new(SharedState {
        package_version,
        main_store: main_store_arc,
        chat_state,
    });

    // --- Middleware ---
    let auth_middleware = middleware::from_fn_with_state(
        shared_state.clone(),
        |State(state): State<Arc<SharedState>>,
         headers: HeaderMap,
         query: Query<CcproxyQuery>,
         req: http::Request<axum::body::Body>,
         next: Next| async move {
            authenticate_request_middleware(State(state), headers, query, req, next, false).await
        },
    );

    let ollama_auth_middleware = middleware::from_fn_with_state(
        shared_state.clone(),
        |State(state): State<Arc<SharedState>>,
         ConnectInfo(addr): ConnectInfo<SocketAddr>,
         req: http::Request<axum::body::Body>,
         next: Next| async move {
            if addr.ip().is_loopback() {
                #[cfg(debug_assertions)]
                log::debug!(
                    "Ollama local access from {}, bypassing authentication",
                    addr.ip()
                );
                return Ok(next.run(req).await);
            }
            #[cfg(debug_assertions)]
            log::debug!(
                "Ollama non-local access from {}, requires authentication",
                addr.ip()
            );
            authenticate_request_middleware(
                State(state),
                req.headers().clone(),
                Query(CcproxyQuery {
                    key: None,
                    _debug: None,
                }),
                req,
                next,
                false,
            )
            .await
        },
    );

    // --- Route Assembly ---

    // 1. Core Protocol Routers
    let normal_routes = Router::new()
        .merge(openai_routes(false, GroupMode::None))
        .merge(claude_routes(false, GroupMode::None))
        .merge(gemini_routes(false, GroupMode::None))
        .merge(decision_routes(GroupMode::None));

    let compat_routes = Router::new()
        .merge(openai_routes(true, GroupMode::None))
        .merge(claude_routes(true, GroupMode::None))
        .merge(gemini_routes(true, GroupMode::None));

    let grouped_normal_routes = Router::new()
        .merge(openai_routes(false, GroupMode::Path))
        .merge(claude_routes(false, GroupMode::Path))
        .merge(gemini_routes(false, GroupMode::Path))
        .merge(decision_routes(GroupMode::Path));

    let grouped_compat_routes = Router::new()
        .merge(openai_routes(true, GroupMode::Path))
        .merge(claude_routes(true, GroupMode::Path))
        .merge(gemini_routes(true, GroupMode::Path));

    // 2. Ollama Routers
    let ollama_normal_chat = ollama_chat_routes(false, GroupMode::None);
    let ollama_compat_chat = ollama_chat_routes(true, GroupMode::None);
    let ollama_grouped_normal_chat = ollama_chat_routes(false, GroupMode::Path);
    let ollama_grouped_compat_chat = ollama_chat_routes(true, GroupMode::Path);

    // 3. MCP & Unauthenticated
    let unauthenticated_router = Router::new()
        .route(
            "/",
            get(|| async { axum::response::Html(include_str!("router_welcome.html")) }),
        )
        .route(
            "/favicon.ico",
            get(|| async { (StatusCode::NOT_FOUND, "") }),
        );

    let new_mcp_router = {
        let http_service = crate::mcp::server::create_http_service(shared_state.chat_state.clone());
        Router::new().nest_service("/http", http_service)
    };

    log_registered_routes();

    // 4. Combined Router with strict priority
    Router::new()
        // A. Static / Root routes (Highest priority, prevents shadowing)
        .merge(unauthenticated_router)
        .merge(ollama_api_routes().layer(ollama_auth_middleware.clone()))
        .nest("/mcp", new_mcp_router)
        // B. Non-prefixed Protocol Routes (Handles /v1, /api, /v1beta directly)
        .merge(normal_routes.layer(auth_middleware.clone()))
        .merge(ollama_normal_chat.layer(ollama_auth_middleware.clone()))
        // C. Compat Mode Routes (Fixed prefix /compat)
        .nest(
            &format!("/{}", TOOL_COMPAT_MODE_PREFIX),
            compat_routes.clone().layer(auth_middleware.clone()),
        )
        .nest(
            &format!("/{}", TOOL_COMPAT_SHORT_PREFIX),
            compat_routes.layer(auth_middleware.clone()),
        )
        .nest(
            &format!("/{}", TOOL_COMPAT_MODE_PREFIX),
            ollama_compat_chat
                .clone()
                .layer(ollama_auth_middleware.clone()),
        )
        .nest(
            &format!("/{}", TOOL_COMPAT_SHORT_PREFIX),
            ollama_compat_chat.layer(ollama_auth_middleware.clone()),
        )
        // D. Grouped Routes (Dynamic prefix /{group_name}, Lowest priority)
        // D1. Grouped + Compat
        .nest(
            &format!("/{{group_name}}/{}", TOOL_COMPAT_MODE_PREFIX),
            grouped_compat_routes.clone().layer(auth_middleware.clone()),
        )
        .nest(
            &format!("/{{group_name}}/{}", TOOL_COMPAT_SHORT_PREFIX),
            grouped_compat_routes.layer(auth_middleware.clone()),
        )
        .nest(
            &format!("/{{group_name}}/{}", TOOL_COMPAT_MODE_PREFIX),
            ollama_grouped_compat_chat
                .clone()
                .layer(ollama_auth_middleware.clone()),
        )
        .nest(
            &format!("/{{group_name}}/{}", TOOL_COMPAT_SHORT_PREFIX),
            ollama_grouped_compat_chat.layer(ollama_auth_middleware.clone()),
        )
        // D2. Grouped Normal (Matches /switch/v1/... or /mygroup/v1/...)
        .nest(
            "/{group_name}",
            grouped_normal_routes.layer(auth_middleware.clone()),
        )
        .nest(
            "/{group_name}",
            ollama_grouped_normal_chat.layer(ollama_auth_middleware.clone()),
        )
        .with_state(shared_state)
}

// ----------------------------------------------------------------------------
// Middleware & Helpers
// ----------------------------------------------------------------------------

/// Middleware for authenticating requests.
fn strip_untrusted_workflow_attribution_headers(headers: &mut HeaderMap) {
    for header in [
        "x-cs-workflow-session-id",
        "x-cs-workflow-task-run-id",
        "x-cs-workflow-segment-id",
        "x-cs-root-session-id",
        "x-cs-root-task-run-id",
        "x-cs-request-kind",
    ] {
        headers.remove(header);
    }
}

async fn authenticate_request_middleware(
    State(state): State<Arc<SharedState>>,
    headers: HeaderMap,
    Query(query): Query<CcproxyQuery>,
    mut req: http::Request<axum::body::Body>,
    next: Next,
    is_local: bool,
) -> Result<Response, Response> {
    let path = req.uri().path().to_string();
    let trusted_internal_request = is_trusted_internal_request(&headers);
    if !trusted_internal_request {
        strip_untrusted_workflow_attribution_headers(req.headers_mut());
    }
    match authenticate_request(
        headers,
        query,
        state.main_store.clone(),
        state.chat_state.clone(),
        is_local,
    )
    .await
    {
        Ok(_) => Ok(next.run(req).await),
        Err(e) => {
            log::warn!("Authentication failed for path {}: {:?}", path, e);
            Err(e.into_response())
        }
    }
}

/// Logs the registered routes for discoverability.
fn log_registered_routes() {
    log::info!("--- ccproxy routes registered ---");
    log::info!("[General]");
    log::info!("  - GET /");
    log::info!("  - GET /api/version");
    log::info!("[OpenAI-Compatible]");
    log::info!("  - /v1/models, /v1/chat/completions, /v1/responses, /v1/embeddings");
    log::info!("[Claude-Compatible]");
    log::info!("  - /v1/messages, /v1/claude/embeddings");
    log::info!("[Gemini-Compatible]");
    log::info!("  - /v1beta/models, /v1beta/models/{{model_id}}:{{action}}");
    log::info!("[System One Decision]");
    log::info!("  - /v1/systemone");
    log::info!("[Ollama-Specific]");
    log::info!("  - /api/tags, /api/show, /api/chat, /api/embeddings, /api/embed");
    log::info!("[Access Modes]");
    log::info!("  - Grouped:           /{{group_name}}/{{path}}");
    log::info!("  - Tool Compat:       /compat/{{path}} or /compat_mode/{{path}}");
    log::info!("  - Grouped + Compat:  /{{group_name}}/compat/{{path}} or /{{group_name}}/compat_mode/{{path}}");
    log::info!("  - Switch:            /switch/{{path}}");
    log::info!("  - Switch + Compat:   /switch/compat/{{path}} or /switch/compat_mode/{{path}}");
    log::info!("-------------------------------------");
    log::info!("[MCP]");
    log::info!("  - MCP Http Streamable Proxy: /mcp/http");
    log::info!("-------------------------------------");
}

#[cfg(test)]
mod usage_attribution_tests {
    use super::*;

    #[test]
    fn authenticated_internal_requests_keep_workflow_attribution_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-cs-internal-request", "true".parse().unwrap());
        headers.insert(
            "authorization",
            format!(
                "Bearer {}",
                crate::constants::INTERNAL_CCPROXY_API_KEY.read()
            )
            .parse()
            .unwrap(),
        );
        headers.insert(
            "x-cs-workflow-session-id",
            "workflow-session".parse().unwrap(),
        );

        assert!(is_trusted_internal_request(&headers));
        assert!(headers.contains_key("x-cs-workflow-session-id"));
    }

    #[test]
    fn external_requests_cannot_preserve_workflow_attribution_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-cs-workflow-session-id",
            "victim-session".parse().unwrap(),
        );
        headers.insert("x-cs-root-task-run-id", "victim-task".parse().unwrap());

        assert!(!is_trusted_internal_request(&headers));
        strip_untrusted_workflow_attribution_headers(&mut headers);

        assert!(!headers.contains_key("x-cs-workflow-session-id"));
        assert!(!headers.contains_key("x-cs-root-task-run-id"));
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    use crate::ai::interaction::chat_completion::ChatState;
    use crate::lifecycle::{RuntimeLifecycleGate, WorkAdmission, WorkKind};
    use crate::libs::window_channels::WindowChannels;
    use axum::body::Body;

    fn shared_state(gate: Option<Arc<RuntimeLifecycleGate>>) -> Arc<SharedState> {
        let store = Arc::new(MainStore::new(":memory:").expect("in-memory store"));
        let chat_state = ChatState::runtime_new(Arc::new(WindowChannels::new()), store.clone());
        if let Some(gate) = gate {
            chat_state.install_admission(gate as Arc<dyn WorkAdmission>);
        }
        Arc::new(SharedState {
            package_version: "test".to_string(),
            main_store: store,
            chat_state,
        })
    }

    fn running_gate() -> (Arc<SharedState>, Arc<RuntimeLifecycleGate>) {
        let gate = Arc::new(RuntimeLifecycleGate::new());
        (shared_state(Some(gate.clone())), gate)
    }

    fn query() -> CcproxyQuery {
        CcproxyQuery {
            key: None,
            _debug: None,
        }
    }

    fn chat_body(stream: bool) -> Bytes {
        Bytes::from(
            String::from(r#"{"model":"alias","stream":"#)
                + if stream { "true" } else { "false" }
                + r#","messages":[{"role":"user","content":"hi"}]}"#,
        )
    }

    async fn status_and_error_code(response: Response) -> (StatusCode, String, String) {
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("json body");
        (
            status,
            value["error"]["code"].as_str().unwrap_or_default().to_string(),
            String::from_utf8_lossy(&body).to_string(),
        )
    }

    /// Direct, adapted and streaming chat work, plus embeddings and the
    /// responses API, all funnel through the one gated logic entry, so a closed
    /// gate refuses each of them with the stable `runtime_stopping` code before
    /// any alias resolution or provider call.
    #[tokio::test]
    async fn a_quiesced_gate_refuses_every_proxy_provider_route() {
        let (state, gate) = running_gate();
        assert!(gate.begin_quiesce());

        let responses = vec![
            // Direct: same-protocol, non-streaming.
            openai_chat_logic(
                state.clone(),
                query(),
                HeaderMap::new(),
                chat_body(false),
                None,
                false,
            )
            .await
            .expect("logic returns a refusal response"),
            // Adapted: cross-protocol client (Claude) onto the unified path.
            claude_chat_logic(
                state.clone(),
                query(),
                HeaderMap::new(),
                chat_body(false),
                None,
                false,
            )
            .await
            .expect("logic returns a refusal response"),
            // Streaming: the request asks for a stream.
            openai_chat_logic(
                state.clone(),
                query(),
                HeaderMap::new(),
                chat_body(true),
                None,
                false,
            )
            .await
            .expect("logic returns a refusal response"),
            gemini_chat_logic(
                state.clone(),
                query(),
                HeaderMap::new(),
                chat_body(false),
                None,
                false,
                "gemini-model".to_string(),
                "streamGenerateContent".to_string(),
            )
            .await
            .expect("logic returns a refusal response"),
            openai_responses_logic(
                state.clone(),
                query(),
                HeaderMap::new(),
                chat_body(false),
                None,
                false,
            )
            .await
            .expect("logic returns a refusal response"),
            openai_embedding_logic(
                state.clone(),
                query(),
                HeaderMap::new(),
                Bytes::from_static(br#"{"model":"alias","input":"hi"}"#),
                None,
            )
            .await
            .expect("logic returns a refusal response"),
            // The single System One decision provider entry.
            decision_logic(
                state.clone(),
                HeaderMap::new(),
                Bytes::from_static(
                    br#"{"model":"alias","state":{},"questions":[]}"#,
                ),
                None,
            )
            .await
            .expect("logic returns a refusal response"),
        ];

        for response in responses {
            let (status, code, body) = status_and_error_code(response).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
            assert_eq!(
                code,
                chatspeed_contracts::RUNTIME_CODE_RUNTIME_STOPPING,
                "{body}"
            );
        }
    }

    /// The single provider entry `/v1/systemone` does real decision-model work,
    /// so a closed gate refuses it with the stable `runtime_stopping` code before
    /// any alias resolution or provider call, exactly like the chat routes.
    #[tokio::test]
    async fn a_quiesced_gate_refuses_the_systemone_decision_entry() {
        let (state, gate) = running_gate();
        assert!(gate.begin_quiesce());

        let response = decision_logic(
            state.clone(),
            HeaderMap::new(),
            Bytes::from_static(br#"{"model":"alias","state":{},"questions":[]}"#),
            None,
        )
        .await
        .expect("logic returns a refusal response");

        let (status, code, body) = status_and_error_code(response).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert_eq!(
            code,
            chatspeed_contracts::RUNTIME_CODE_RUNTIME_STOPPING,
            "{body}"
        );
    }

    /// Read-only model listing is never gated, so it stays readable while the
    /// runtime is quiescing.
    #[tokio::test]
    async fn a_quiesced_gate_leaves_read_only_model_listing_readable() {
        let (state, gate) = running_gate();
        assert!(gate.begin_quiesce());

        let response = match openai_list_models_logic(state.clone(), None).await {
            Ok(response) => response,
            Err(error) => error.into_response(),
        };
        let (status, code, _body) = status_and_error_code(response).await;
        assert_ne!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_ne!(code, chatspeed_contracts::RUNTIME_CODE_RUNTIME_STOPPING);
    }

    /// While the runtime is running, the gate is transparent: the same request
    /// produces byte-identical status, headers and body with and without a gate,
    /// and the running admission is released with the buffered response.
    #[tokio::test]
    async fn a_running_gate_does_not_change_a_buffered_proxy_response() {
        let (gated_state, gate) = running_gate();
        let plain_state = shared_state(None);

        let gated = openai_chat_logic(
            gated_state.clone(),
            query(),
            HeaderMap::new(),
            chat_body(false),
            None,
            false,
        )
        .await
        .expect("gated response");
        let plain = openai_chat_logic(
            plain_state.clone(),
            query(),
            HeaderMap::new(),
            chat_body(false),
            None,
            false,
        )
        .await
        .expect("plain response");

        assert_eq!(gated.status(), plain.status());
        assert_eq!(gated.headers(), plain.headers());
        let gated_body = axum::body::to_bytes(gated.into_body(), usize::MAX)
            .await
            .expect("gated body");
        let plain_body = axum::body::to_bytes(plain.into_body(), usize::MAX)
            .await
            .expect("plain body");
        assert_eq!(gated_body, plain_body);

        assert_eq!(
            gate.active_admissions(),
            0,
            "the buffered response released its running admission"
        );
    }

    /// Through the `gated` seam itself, a running gate passes a handler's status,
    /// headers and body through unchanged and releases a buffered admission.
    #[tokio::test]
    async fn gated_preserves_a_running_handlers_status_headers_and_body() {
        let (state, gate) = running_gate();

        let response = gated(&state, WorkKind::ChatTurn, async {
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("x-upstream", "kept")
                .header("content-type", "application/json")
                .body(Body::from(Bytes::from_static(b"{\"ok\":true}")))
                .expect("build response"))
        })
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("x-upstream").unwrap(), "kept");
        // The exact length hint survives, so the framework keeps the framing.
        assert_eq!(response.body().size_hint().exact(), Some(11));
        assert_eq!(gate.active_admissions(), 0);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("consume the body");
        assert_eq!(body, Bytes::from_static(b"{\"ok\":true}"));
    }

    /// Through the `gated` seam itself, a running gate preserves a streaming
    /// handler's headers and holds the admission until the stream ends.
    #[tokio::test]
    async fn gated_preserves_a_running_streaming_handlers_headers_and_holds_its_admission() {
        let (state, gate) = running_gate();
        let chunks: Vec<Result<Bytes, std::io::Error>> = vec![
            Ok(Bytes::from_static(b"data: a\n\n")),
            Ok(Bytes::from_static(b"data: b\n\n")),
        ];

        let response = gated(&state, WorkKind::ChatTurn, async move {
            Ok(Response::builder()
                .header("content-type", "text/event-stream")
                .header("x-upstream", "kept")
                .body(Body::from_stream(futures_util::stream::iter(chunks)))
                .expect("build response"))
        })
        .await;

        assert_eq!(response.headers().get("x-upstream").unwrap(), "kept");
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "text/event-stream"
        );
        assert_eq!(
            gate.active_admissions(),
            1,
            "a streaming response is still in flight"
        );

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("consume the body");
        assert_eq!(body, Bytes::from_static(b"data: a\n\ndata: b\n\n"));
        assert_eq!(
            gate.active_admissions(),
            0,
            "the admission is released once the stream ended"
        );
    }

    /// A streaming body keeps producing bytes after the handler returned, so the
    /// admission must stay held until those bytes stop.
    #[tokio::test]
    async fn hold_admission_keeps_a_streaming_body_counted_until_the_body_ends() {
        let gate = Arc::new(RuntimeLifecycleGate::new());
        let chunks: Vec<Result<Bytes, std::io::Error>> = vec![
            Ok(Bytes::from_static(b"a")),
            Ok(Bytes::from_static(b"b")),
        ];
        let response = Response::new(Body::from_stream(futures_util::stream::iter(chunks)));
        let ticket = gate.admit(WorkKind::ChatTurn).expect("admit");
        assert_eq!(gate.active_admissions(), 1);

        let response = hold_admission(response, ticket);
        assert_eq!(
            gate.active_admissions(),
            1,
            "the admission is held while the stream lives"
        );

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("consume the body");
        assert_eq!(body, Bytes::from_static(b"ab"));
        assert_eq!(
            gate.active_admissions(),
            0,
            "the admission is released once the body ended"
        );
    }

    /// A buffered body carries an exact length hint, so it is released with the
    /// handler and its framing is left untouched (no switch to chunked).
    #[tokio::test]
    async fn hold_admission_leaves_a_buffered_body_and_its_framing_untouched() {
        let gate = Arc::new(RuntimeLifecycleGate::new());
        let response = Response::builder()
            .header("x-kept", "1")
            .body(Body::from(Bytes::from_static(b"{\"ok\":true}")))
            .expect("build response");
        let ticket = gate.admit(WorkKind::ChatTurn).expect("admit");

        let response = hold_admission(response, ticket);
        assert_eq!(response.body().size_hint().exact(), Some(11));
        assert_eq!(response.headers().get("x-kept").unwrap(), "1");
        assert_eq!(
            gate.active_admissions(),
            0,
            "a buffered body has no provider work left"
        );

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("consume the body");
        assert_eq!(body, Bytes::from_static(b"{\"ok\":true}"));
    }
}
