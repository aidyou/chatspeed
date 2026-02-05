//! This module defines the routing for the `ccproxy` server. It handles dispatching
//! incoming HTTP requests for various AI models (OpenAI, Claude, Gemini, Ollama)
//! to the appropriate backend logic.
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
//! ### Grouped Model Access
//! `ccproxy` allows organizing models into groups (e.g., `qwen`). This is useful for routing
//! requests to a specific set of models.
//!
//! To access a model in a group, prefix the standard API path with the group name, like `/qwen`.
//!
//! # API Endpoints
//!
//! The following sections detail the available API endpoints. Both Tool Compatibility Mode and
//! Grouped Model Access can be combined where applicable.
//!
//! ### General Endpoints
//! - `GET /`: Returns a welcome message from the ccproxy.
//! - `GET /api/version`: Shows the current `ccproxy` version.
//!
//! ### OpenAI-Compatible Endpoints
//! These endpoints mimic the OpenAI API.
//! - `GET /v1/models`: Lists available models.
//!   - *Grouped Example*: `GET /qwen/v1/models`
//! - `POST /v1/chat/completions`: Creates a chat completion.
//!   - *Tool Compat Example*: `POST /compat_mode/v1/chat/completions`
//!   - *Grouped Example*: `POST /qwen/v1/chat/completions`
//!   - *Combined Example*: `POST /qwen/compat_mode/v1/chat/completions`
//! - `POST /v1/completions`: (Legacy) Creates a text completion.
//! - `POST /v1/embeddings`: Creates an embedding vector for a given input.
//!
//! ### Claude-Compatible Endpoints
//! - `POST /v1/messages`: Creates a message with the Claude model.
//!
//! ### Gemini-Compatible Endpoints
//! - `GET /v1beta/models`: Lists available Gemini models.
//! - `POST /v1beta/models/{model_id}:{action}`: Generates content.
//!   - `{model_id}`: The name of the model (e.g., `gemini2.5-flash`).
//!   - `{action}`: The generation method (e.g., `generateContent`, `streamGenerateContent`).
//!
//! ### Ollama-Specific Endpoints
//! - `GET /api/tags`: Lists local Ollama models.
//! - `POST /api/show`: Returns information about a specific Ollama model.
//! - `POST /api/chat`: Creates a chat completion with an Ollama model.
//!
//! # Internal Architecture
//!
//! 1.  **Individual Chat Logic Handlers**:
//!     -   `openai_chat_logic`, `claude_chat_logic`, `gemini_chat_logic`, `ollama_chat_logic`
//!         contain the business logic for interacting with each specific AI service.
//!
//! 2.  **Model-Specific Route Definitions**:
//!     -   `openai_routes`, `claude_routes`, etc., define the Axum `Router` for each model type.
//!
//! 3.  **Authentication Middleware**:
//!     -   `authenticate_request_middleware` is used to protect endpoints and ensure valid credentials.
//!
//! 4.  **Main Router Composition**:
//!     -   The `routes` function assembles all the individual routers into a single, unified service,
//!         applying middleware as needed. This modular approach allows for easy expansion.
//!

use crate::ai::interaction::chat_completion::ChatState;
use crate::ccproxy::errors::CCProxyError;
use crate::ccproxy::ChatProtocol;
use crate::ccproxy::{
    auth::authenticate_request,
    handle_chat_completion, handle_embedding, handle_list_models, handle_ollama_tags,
    handler::{handle_gemini_list_models, handle_ollama_show, ollama_extra_handler::ShowRequest},
    helper::CcproxyQuery,
};
use crate::constants::CFG_ACTIVE_PROXY_GROUP;
use crate::db::MainStore;

use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{self, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use bytes::Bytes;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;

const TOOL_COMPAT_MODE_PREFIX: &str = "compat_mode";
const TOOL_COMPAT_SHORT_PREFIX: &str = "compat";
const SWITCH_MODE_PREFIX: &str = "switch";

// A struct to hold the shared state, which is passed to all route handlers.
pub struct SharedState {
    pub app_handle: tauri::AppHandle,
    pub main_store: Arc<std::sync::RwLock<MainStore>>,
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
            state.main_store.read().ok().and_then(|store| {
                store
                    .get_config(CFG_ACTIVE_PROXY_GROUP, "default".to_string())
                    .into()
            })
        } else {
            Some(g)
        }
    } else {
        None
    }
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

    Ok(handle_chat_completion(
        ChatProtocol::OpenAI,
        headers,
        query,
        body,
        final_group,
        compat_mode,
        "".to_string(), // model_id
        "".to_string(), // action
        state.main_store.clone(),
    )
    .await
    .into_response())
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

    Ok(handle_chat_completion(
        ChatProtocol::Claude,
        headers,
        query,
        body,
        final_group,
        compat_mode,
        "".to_string(), // model_id
        "".to_string(), // action
        state.main_store.clone(),
    )
    .await
    .into_response())
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

    Ok(handle_chat_completion(
        ChatProtocol::Gemini,
        headers,
        query,
        body,
        final_group,
        compat_mode,
        model_id,
        action,
        state.main_store.clone(),
    )
    .await
    .into_response())
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

    Ok(handle_chat_completion(
        ChatProtocol::Ollama,
        headers,
        query,
        body,
        final_group,
        compat_mode,
        "".to_string(), // model_id
        "".to_string(), // action
        state.main_store.clone(),
    )
    .await
    .into_response())
}

async fn openai_embedding_logic(
    state: Arc<SharedState>,
    query: CcproxyQuery,
    headers: HeaderMap,
    body: Bytes,
    group_name: Option<String>,
) -> Result<Response, CCProxyError> {
    let final_group = resolve_group_name(&state, group_name);

    Ok(handle_embedding(
        ChatProtocol::OpenAI,
        headers,
        query,
        body,
        final_group,
        "".to_string(),
        state.main_store.clone(),
    )
    .await
    .into_response())
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

    Ok(handle_embedding(
        ChatProtocol::Gemini,
        headers,
        query,
        body,
        final_group,
        model_id,
        state.main_store.clone(),
    )
    .await
    .into_response())
}

async fn ollama_embedding_logic(
    state: Arc<SharedState>,
    query: CcproxyQuery,
    headers: HeaderMap,
    body: Bytes,
    group_name: Option<String>,
) -> Result<Response, CCProxyError> {
    let final_group = resolve_group_name(&state, group_name);

    Ok(handle_embedding(
        ChatProtocol::Ollama,
        headers,
        query,
        body,
        final_group,
        "".to_string(),
        state.main_store.clone(),
    )
    .await
    .into_response())
}

async fn claude_embedding_logic(
    state: Arc<SharedState>,
    query: CcproxyQuery,
    headers: HeaderMap,
    body: Bytes,
    group_name: Option<String>,
) -> Result<Response, CCProxyError> {
    let final_group = resolve_group_name(&state, group_name);

    Ok(handle_embedding(
        ChatProtocol::Claude,
        headers,
        query,
        body,
        final_group,
        "".to_string(),
        state.main_store.clone(),
    )
    .await
    .into_response())
}

// ----------------------------------------------------------------------------
// Protocol-specific Route Builders
// ----------------------------------------------------------------------------

enum GroupMode {
    None,
    Path,
    Fixed(String),
}

/// Creates routes for OpenAI-compatible endpoints.
fn openai_routes(compat_mode: bool, mode: GroupMode) -> Router<Arc<SharedState>> {
    match mode {
        GroupMode::Fixed(fixed_group) => {
            let fixed_group_for_chat = fixed_group.clone();
            let chat_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    openai_chat_logic(
                        state,
                        query,
                        headers,
                        body,
                        Some(fixed_group_for_chat.clone()),
                        compat_mode,
                    )
                    .await
                    .map_err(|e| e.into_response())
                },
            );
            let fixed_group_for_list = fixed_group.clone();
            let list_model_handler = get(move |State(state): State<Arc<SharedState>>| async move {
                let final_group = resolve_group_name(&state, Some(fixed_group_for_list.clone()));
                handle_list_models(final_group, state.main_store.clone())
                    .await
                    .map_err(|e| e.into_response())
            });
            let fixed_group_for_embed = fixed_group.clone();
            let embedding_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    openai_embedding_logic(
                        state,
                        query,
                        headers,
                        body,
                        Some(fixed_group_for_embed.clone()),
                    )
                    .await
                    .map_err(|e| e.into_response())
                },
            );
            Router::new()
                .route("/v1/chat/completions", chat_handler)
                .route("/v1/models", list_model_handler)
                .route("/v1/embeddings", embedding_handler)
        }
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
            let list_model_handler = get(
                |State(state): State<Arc<SharedState>>, Path(group_name): Path<String>| async move {
                    handle_list_models(Some(group_name), state.main_store.clone())
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
            let list_model_handler = get(|State(state): State<Arc<SharedState>>| async move {
                handle_list_models(None, state.main_store.clone())
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
                .route("/v1/models", list_model_handler)
                .route("/v1/embeddings", embedding_handler)
        }
    }
}

/// Creates routes for Claude-compatible endpoints.
fn claude_routes(compat_mode: bool, mode: GroupMode) -> Router<Arc<SharedState>> {
    match mode {
        GroupMode::Fixed(fixed_group) => {
            let fixed_group_for_chat = fixed_group.clone();
            let chat_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    claude_chat_logic(
                        state,
                        query,
                        headers,
                        body,
                        Some(fixed_group_for_chat.clone()),
                        compat_mode,
                    )
                    .await
                    .map_err(|e| e.into_response())
                },
            );
            let fixed_group_for_embed = fixed_group.clone();
            let embedding_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    claude_embedding_logic(
                        state,
                        query,
                        headers,
                        body,
                        Some(fixed_group_for_embed.clone()),
                    )
                    .await
                    .map_err(|e| e.into_response())
                },
            );
            Router::new()
                .route("/v1/messages", chat_handler)
                .route("/v1/claude/embeddings", embedding_handler)
        }
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
        GroupMode::Fixed(fixed_group) => {
            let fixed_group_for_chat = fixed_group.clone();
            let chat_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Path(action): Path<String>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    let (model_id, action) = action
                        .rsplit_once(':')
                        .ok_or_else(|| CCProxyError::InvalidProtocolError(action.clone()))
                        .map_err(|e| e.into_response())?;
                    if action == "embedContent" {
                        return gemini_embedding_logic(
                            state,
                            query,
                            headers,
                            body,
                            Some(fixed_group_for_chat.clone()),
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
                        Some(fixed_group_for_chat.clone()),
                        compat_mode,
                        model_id.to_string(),
                        action.to_string(),
                    )
                    .await
                    .map_err(|e| e.into_response())
                },
            );
            let fixed_group_for_list = fixed_group.clone();
            let list_model_handler = get(move |State(state): State<Arc<SharedState>>| async move {
                let final_group = resolve_group_name(&state, Some(fixed_group_for_list.clone()));
                handle_gemini_list_models(final_group, state.main_store.clone())
                    .await
                    .map_err(|e| e.into_response())
            });
            Router::new()
                .route("/v1beta/models/{action}", chat_handler)
                .route("/v1beta/models", list_model_handler)
        }
        GroupMode::Path => {
            let chat_handler = post(
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
                    handle_gemini_list_models(Some(group_name), state.main_store.clone())
                        .await
                        .map_err(|e| e.into_response())
                },
            );
            Router::new()
                .route("/v1beta/models/{model_and_action}", chat_handler)
                .route("/v1beta/models", list_model_handler)
        }
        GroupMode::None => {
            let chat_handler = post(
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
                handle_gemini_list_models(None, state.main_store.clone())
                    .await
                    .map_err(|e| e.into_response())
            });
            Router::new()
                .route("/v1beta/models/{model_and_action}", chat_handler)
                .route("/v1beta/models", list_model_handler)
        }
    }
}

/// Creates chat routes for Ollama-compatible endpoints.
fn ollama_chat_routes(compat_mode: bool, mode: GroupMode) -> Router<Arc<SharedState>> {
    match mode {
        GroupMode::Fixed(fixed_group) => {
            let fixed_group_for_chat = fixed_group.clone();
            let chat_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    ollama_chat_logic(
                        state,
                        query,
                        headers,
                        body,
                        Some(fixed_group_for_chat.clone()),
                        compat_mode,
                    )
                    .await
                    .map_err(|e| e.into_response())
                },
            );
            let fixed_group_for_list = fixed_group.clone();
            let list_model_handler = get(move |State(state): State<Arc<SharedState>>| async move {
                let final_group = resolve_group_name(&state, Some(fixed_group_for_list.clone()));
                handle_ollama_tags(final_group, state.main_store.clone())
                    .await
                    .map_err(|e| e.into_response())
            });
            let fixed_group_for_embed = fixed_group.clone();
            let embedding_handler = post(
                move |State(state): State<Arc<SharedState>>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    ollama_embedding_logic(
                        state,
                        query,
                        headers,
                        body,
                        Some(fixed_group_for_embed.clone()),
                    )
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
                    handle_ollama_tags(Some(group_name), state.main_store.clone())
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
                handle_ollama_tags(None, state.main_store.clone())
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
                let version = state.app_handle.package_info().version.to_string();
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

// ----------------------------------------------------------------------------
// Main Router Definition
// ----------------------------------------------------------------------------

/// Defines all routes for the ccproxy module.
pub async fn routes(
    app_handle: tauri::AppHandle,
    main_store_arc: Arc<std::sync::RwLock<MainStore>>,
    chat_state: Arc<ChatState>,
) -> Router {
    let shared_state = Arc::new(SharedState {
        app_handle,
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
                    debug: None,
                }),
                req,
                next,
                false,
            )
            .await
        },
    );

    // --- Route Assembly ---
    // 1. Build routers for authenticated, non-Ollama protocols
    let normal_routes = Router::new()
        .merge(openai_routes(false, GroupMode::None))
        .merge(claude_routes(false, GroupMode::None))
        .merge(gemini_routes(false, GroupMode::None));

    let grouped_normal_routes = Router::new()
        .merge(openai_routes(false, GroupMode::Path))
        .merge(claude_routes(false, GroupMode::Path))
        .merge(gemini_routes(false, GroupMode::Path));

    let switch_normal_routes = Router::new()
        .merge(openai_routes(
            false,
            GroupMode::Fixed(SWITCH_MODE_PREFIX.to_string()),
        ))
        .merge(claude_routes(
            false,
            GroupMode::Fixed(SWITCH_MODE_PREFIX.to_string()),
        ))
        .merge(gemini_routes(
            false,
            GroupMode::Fixed(SWITCH_MODE_PREFIX.to_string()),
        ));

    let compat_routes = Router::new()
        .merge(openai_routes(true, GroupMode::None))
        .merge(claude_routes(true, GroupMode::None))
        .merge(gemini_routes(true, GroupMode::None));

    let grouped_compat_routes = Router::new()
        .merge(openai_routes(true, GroupMode::Path))
        .merge(claude_routes(true, GroupMode::Path))
        .merge(gemini_routes(true, GroupMode::Path));

    let switch_compat_routes = Router::new()
        .merge(openai_routes(
            true,
            GroupMode::Fixed(SWITCH_MODE_PREFIX.to_string()),
        ))
        .merge(claude_routes(
            true,
            GroupMode::Fixed(SWITCH_MODE_PREFIX.to_string()),
        ))
        .merge(gemini_routes(
            true,
            GroupMode::Fixed(SWITCH_MODE_PREFIX.to_string()),
        ));

    // 2. Build routers for Ollama chat
    let ollama_normal_chat = ollama_chat_routes(false, GroupMode::None);
    let ollama_grouped_normal_chat = ollama_chat_routes(false, GroupMode::Path);
    let ollama_switch_normal_chat =
        ollama_chat_routes(false, GroupMode::Fixed(SWITCH_MODE_PREFIX.to_string()));
    let ollama_compat_chat = ollama_chat_routes(true, GroupMode::None);
    let ollama_grouped_compat_chat = ollama_chat_routes(true, GroupMode::Path);
    let ollama_switch_compat_chat =
        ollama_chat_routes(true, GroupMode::Fixed(SWITCH_MODE_PREFIX.to_string()));

    // 3. Unauthenticated routes
    let unauthenticated_router = Router::new()
        .route("/", get(|| async { "Chatspeed ccproxy is running." }))
        .route(
            "/favicon.ico",
            get(|| async { (StatusCode::NOT_FOUND, "") }),
        );

    log_registered_routes();

    // 4. Build and combine MCP routers
    let new_mcp_router = {
        let sse_router =
            crate::mcp::server::create_sse_router(shared_state.chat_state.clone(), None);
        let http_service = crate::mcp::server::create_http_service(shared_state.chat_state.clone());
        Router::new()
            .nest_service("/sse", sse_router)
            .nest_service("/http", http_service)
    };
    let legacy_sse_router =
        crate::mcp::server::create_sse_router(shared_state.chat_state.clone(), None);

    Router::new()
        .merge(unauthenticated_router)
        // Standalone Ollama API routes (no grouping/compat)
        .merge(ollama_api_routes().layer(ollama_auth_middleware.clone()))
        // MCP Routes
        .nest("/mcp", new_mcp_router)
        .nest_service("/sse", legacy_sse_router) // Use nest_service to handle different state types
        // --- Route Combination Logic ---
        // 1. Switch mode with compatibility (Highest priority)
        .nest(
            &format!("/{}/{}", SWITCH_MODE_PREFIX, TOOL_COMPAT_MODE_PREFIX),
            switch_compat_routes.clone().layer(auth_middleware.clone()),
        )
        .nest(
            &format!("/{}/{}", SWITCH_MODE_PREFIX, TOOL_COMPAT_SHORT_PREFIX),
            switch_compat_routes.clone().layer(auth_middleware.clone()),
        )
        .nest(
            &format!("/{}/{}", SWITCH_MODE_PREFIX, TOOL_COMPAT_MODE_PREFIX),
            ollama_switch_compat_chat
                .clone()
                .layer(ollama_auth_middleware.clone()),
        )
        .nest(
            &format!("/{}/{}", SWITCH_MODE_PREFIX, TOOL_COMPAT_SHORT_PREFIX),
            ollama_switch_compat_chat
                .clone()
                .layer(ollama_auth_middleware.clone()),
        )
        // 2. Switch mode
        .nest(
            &format!("/{}", SWITCH_MODE_PREFIX),
            switch_normal_routes.layer(auth_middleware.clone()),
        )
        .nest(
            &format!("/{}", SWITCH_MODE_PREFIX),
            ollama_switch_normal_chat.layer(ollama_auth_middleware.clone()),
        )
        // 3. Non-grouped, compatibility mode
        .nest(
            &format!("/{}", TOOL_COMPAT_MODE_PREFIX),
            compat_routes.clone().layer(auth_middleware.clone()),
        )
        .nest(
            &format!("/{}", TOOL_COMPAT_SHORT_PREFIX),
            compat_routes.clone().layer(auth_middleware.clone()),
        )
        .nest(
            &format!("/{}", TOOL_COMPAT_MODE_PREFIX),
            ollama_compat_chat
                .clone()
                .layer(ollama_auth_middleware.clone()),
        )
        .nest(
            &format!("/{}", TOOL_COMPAT_SHORT_PREFIX),
            ollama_compat_chat
                .clone()
                .layer(ollama_auth_middleware.clone()),
        )
        // 4. Grouped, compatibility mode
        .nest(
            &format!("/{{group_name}}/{}", TOOL_COMPAT_MODE_PREFIX),
            grouped_compat_routes.clone().layer(auth_middleware.clone()),
        )
        .nest(
            &format!("/{{group_name}}/{}", TOOL_COMPAT_SHORT_PREFIX),
            grouped_compat_routes.clone().layer(auth_middleware.clone()),
        )
        .nest(
            &format!("/{{group_name}}/{}", TOOL_COMPAT_MODE_PREFIX),
            ollama_grouped_compat_chat
                .clone()
                .layer(ollama_auth_middleware.clone()),
        )
        .nest(
            &format!("/{{group_name}}/{}", TOOL_COMPAT_SHORT_PREFIX),
            ollama_grouped_compat_chat
                .clone()
                .layer(ollama_auth_middleware.clone()),
        )
        // 5. Non-grouped, normal mode
        .merge(normal_routes.clone().layer(auth_middleware.clone()))
        .merge(
            ollama_normal_chat
                .clone()
                .layer(ollama_auth_middleware.clone()),
        )
        // 6. Grouped, normal mode (Lowest priority)
        .nest(
            "/{group_name}",
            grouped_normal_routes.clone().layer(auth_middleware.clone()),
        )
        .nest(
            "/{group_name}",
            ollama_grouped_normal_chat
                .clone()
                .layer(ollama_auth_middleware.clone()),
        )
        .with_state(shared_state)
}

// ----------------------------------------------------------------------------
// Middleware & Helpers
// ----------------------------------------------------------------------------

/// Middleware for authenticating requests.
async fn authenticate_request_middleware(
    State(state): State<Arc<SharedState>>,
    headers: HeaderMap,
    Query(query): Query<CcproxyQuery>,
    req: http::Request<axum::body::Body>,
    next: Next,
    is_local: bool,
) -> Result<Response, Response> {
    let path = req.uri().path().to_string();
    match authenticate_request(headers, query, state.main_store.clone(), is_local).await {
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
    log::info!("  - /v1/models, /v1/chat/completions, /v1/embeddings");
    log::info!("[Claude-Compatible]");
    log::info!("  - /v1/messages, /v1/claude/embeddings");
    log::info!("[Gemini-Compatible]");
    log::info!("  - /v1beta/models, /v1beta/models/{{model_id}}:{{action}}");
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
    log::warn!("  - MCP SSE Proxy: /sse <- deprecated");
    log::info!("  - MCP SSE Proxy: /mcp/sse ");
    log::info!("  - MCP Http Streamable Proxy: /mcp/http");
    log::info!("-------------------------------------");
}
