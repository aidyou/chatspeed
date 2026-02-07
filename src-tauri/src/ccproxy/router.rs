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
//! ### Integrated Module Endpoints (Non-ccproxy core)
//! These routes are integrated into this router for unified access but handled by separate modules:
//! - **MCP (Model Context Protocol)**:
//!   - `POST /mcp/http`: Streamable HTTP entry (Recommended).
//!   - `GET /mcp/sse`: SSE entry (Legacy).
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
//!         **Fixed Prefixes** (MCP/SSE) > **Direct Routes** (/v1) > **Global Compat** (/compat) > **Grouped Routes** (/{group}).
//!     -   This ordering prevents route shadowing, ensuring that static protocol paths are
//!         not misidentified as dynamic group names.
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

    // 1. Core Protocol Routers
    let normal_routes = Router::new()
        .merge(openai_routes(false, GroupMode::None))
        .merge(claude_routes(false, GroupMode::None))
        .merge(gemini_routes(false, GroupMode::None));

    let compat_routes = Router::new()
        .merge(openai_routes(true, GroupMode::None))
        .merge(claude_routes(true, GroupMode::None))
        .merge(gemini_routes(true, GroupMode::None));

    let grouped_normal_routes = Router::new()
        .merge(openai_routes(false, GroupMode::Path))
        .merge(claude_routes(false, GroupMode::Path))
        .merge(gemini_routes(false, GroupMode::Path));

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
        .route("/", get(|| async { "Chatspeed ccproxy is running." }))
        .route(
            "/favicon.ico",
            get(|| async { (StatusCode::NOT_FOUND, "") }),
        );

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

    log_registered_routes();

    // 4. Combined Router with strict priority
    Router::new()
        // A. Static / Root routes (Highest priority, prevents shadowing)
        .merge(unauthenticated_router)
        .merge(ollama_api_routes().layer(ollama_auth_middleware.clone()))
        .nest("/mcp", new_mcp_router)
        .nest_service("/sse", legacy_sse_router)
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
    log::warn!("  - MCP SSE Proxy: /mcp/sse <- deprecated");
    log::info!("  - MCP Http Streamable Proxy: /mcp/http");
    log::info!("-------------------------------------");
}
