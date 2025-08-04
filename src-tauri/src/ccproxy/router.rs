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
//! To use this mode, prefix the standard API path with `/compat_mode`.
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

use crate::ai::interaction::{chat_completion::ChatState, ChatProtocol};
use crate::ccproxy::{
    auth::authenticate_request,
    handle_chat_completion, handle_ollama_tags, handle_openai_list_models,
    handler::{handle_gemini_list_models, handle_ollama_show, ollama_extra_handler::ShowRequest},
    helper::CcproxyQuery,
};
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
use std::sync::{Arc, Mutex};

const TOOL_COMPAT_MODE_PREFIX: &str = "compat_mode";

// A struct to hold the shared state, which is passed to all route handlers.
pub struct SharedState {
    pub app_handle: tauri::AppHandle,
    pub main_store: Arc<Mutex<MainStore>>,
    pub chat_state: Arc<ChatState>,
}

// ----------------------------------------------------------------------------
// Core Logic Handlers
// ----------------------------------------------------------------------------
// These handlers contain the actual business logic. They are called by the
// axum handlers below and are agnostic of the routing layer.

// TODO: All `*_logic` functions call `handle_openai_chat_completion`.
// You need to update its signature to accept `group_name: String` as a new argument.
// I have added it to the call sites with a placeholder.

async fn openai_chat_logic(
    state: Arc<SharedState>,
    query: CcproxyQuery,
    headers: HeaderMap,
    body: Bytes,
    group_name: Option<String>,
    compat_mode: bool,
) -> Response {
    handle_chat_completion(
        ChatProtocol::OpenAI,
        headers,
        query,
        body,
        group_name,
        compat_mode,
        "".to_string(), // model_id
        "".to_string(), // action
        state.main_store.clone(),
    )
    .await
    .into_response()
}

async fn claude_chat_logic(
    state: Arc<SharedState>,
    query: CcproxyQuery,
    headers: HeaderMap,
    body: Bytes,
    group_name: Option<String>,
    compat_mode: bool,
) -> Response {
    handle_chat_completion(
        ChatProtocol::Claude,
        headers,
        query,
        body,
        group_name,
        compat_mode,
        "".to_string(), // model_id
        "".to_string(), // action
        state.main_store.clone(),
    )
    .await
    .into_response()
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
) -> Response {
    handle_chat_completion(
        ChatProtocol::Gemini,
        headers,
        query,
        body,
        group_name,
        compat_mode,
        model_id,
        action,
        state.main_store.clone(),
    )
    .await
    .into_response()
}

async fn ollama_chat_logic(
    state: Arc<SharedState>,
    query: CcproxyQuery,
    headers: HeaderMap,
    body: Bytes,
    group_name: Option<String>,
    compat_mode: bool,
) -> Response {
    handle_chat_completion(
        ChatProtocol::Ollama,
        headers,
        query,
        body,
        group_name,
        compat_mode,
        "".to_string(), // model_id
        "".to_string(), // action
        state.main_store.clone(),
    )
    .await
    .into_response()
}

// ----------------------------------------------------------------------------
// Protocol-specific Route Builders
// ----------------------------------------------------------------------------

/// Creates routes for OpenAI-compatible endpoints.
fn openai_routes(compat_mode: bool, grouped: bool) -> Router<Arc<SharedState>> {
    let (chat_handler, list_model_handler) = if grouped {
        (
            post(
                move |State(state): State<Arc<SharedState>>,
                      Path(group_name): Path<String>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    openai_chat_logic(state, query, headers, body, Some(group_name), compat_mode)
                        .await
                },
            ),
            get(
                |State(state): State<Arc<SharedState>>, Path(group_name): Path<String>| async move {
                    handle_openai_list_models(Some(group_name), state.main_store.clone())
                        .await
                        .into_response()
                },
            ),
        )
    } else {
        (
            post(
                move |State(state): State<Arc<SharedState>>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    openai_chat_logic(state, query, headers, body, None, compat_mode).await
                },
            ),
            get(|State(state): State<Arc<SharedState>>| async move {
                handle_openai_list_models(None, state.main_store.clone())
                    .await
                    .into_response()
            }),
        )
    };

    Router::new()
        .route("/v1/chat/completions", chat_handler)
        .route("/v1/models", list_model_handler)
}

/// Creates routes for Claude-compatible endpoints.
fn claude_routes(compat_mode: bool, grouped: bool) -> Router<Arc<SharedState>> {
    let handler = if grouped {
        post(
            move |State(state): State<Arc<SharedState>>,
                  Path(group_name): Path<String>,
                  Query(query): Query<CcproxyQuery>,
                  headers: HeaderMap,
                  body: Bytes| async move {
                claude_chat_logic(state, query, headers, body, Some(group_name), compat_mode).await
            },
        )
    } else {
        post(
            move |State(state): State<Arc<SharedState>>,
                  Query(query): Query<CcproxyQuery>,
                  headers: HeaderMap,
                  body: Bytes| async move {
                claude_chat_logic(state, query, headers, body, None, compat_mode).await
            },
        )
    };
    Router::new().route("/v1/messages", handler)
}

/// Creates routes for Gemini-compatible endpoints.
fn gemini_routes(compat_mode: bool, grouped: bool) -> Router<Arc<SharedState>> {
    let (chat_handler, list_model_handler) = if grouped {
        (
            post(
                move |State(state): State<Arc<SharedState>>,
                      Path((group_name, model_id, action)): Path<(String, String, String)>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
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
                },
            ),
            get(
                |State(state): State<Arc<SharedState>>, Path(group_name): Path<String>| async move {
                    handle_gemini_list_models(Some(group_name), state.main_store.clone()).await
                },
            ),
        )
    } else {
        (
            post(
                move |State(state): State<Arc<SharedState>>,
                      Path((model_id, action)): Path<(String, String)>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
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
                },
            ),
            get(|State(state): State<Arc<SharedState>>| async move {
                handle_gemini_list_models(None, state.main_store.clone())
                    .await
                    .into_response()
            }),
        )
    };

    Router::new()
        .route("/v1beta/models/{model_id}/{action}", chat_handler)
        .route("/v1beta/models", list_model_handler)
}

/// Creates chat routes for Ollama-compatible endpoints.
fn ollama_chat_routes(compat_mode: bool, grouped: bool) -> Router<Arc<SharedState>> {
    let (chat_handler, list_model_handler) = if grouped {
        (
            post(
                move |State(state): State<Arc<SharedState>>,
                      Path(group_name): Path<String>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    ollama_chat_logic(state, query, headers, body, Some(group_name), compat_mode)
                        .await
                },
            ),
            get(
                |State(state): State<Arc<SharedState>>, Path(group_name): Path<String>| async move {
                    handle_ollama_tags(Some(group_name), state.main_store.clone())
                        .await
                        .into_response()
                },
            ),
        )
    } else {
        (
            post(
                move |State(state): State<Arc<SharedState>>,
                      Query(query): Query<CcproxyQuery>,
                      headers: HeaderMap,
                      body: Bytes| async move {
                    ollama_chat_logic(state, query, headers, body, None, compat_mode).await
                },
            ),
            get(|State(state): State<Arc<SharedState>>| async move {
                handle_ollama_tags(None, state.main_store.clone())
                    .await
                    .into_response()
            }),
        )
    };
    Router::new()
        .route("/api/chat", chat_handler)
        .route("/api/tags", list_model_handler)
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
                        .into_response()
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
    main_store_arc: Arc<Mutex<MainStore>>,
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
        .merge(openai_routes(false, false))
        .merge(claude_routes(false, false))
        .merge(gemini_routes(false, false));

    let grouped_normal_routes = Router::new()
        .merge(openai_routes(false, true))
        .merge(claude_routes(false, true))
        .merge(gemini_routes(false, true));

    let compat_routes = Router::new()
        .merge(openai_routes(true, false))
        .merge(claude_routes(true, false))
        .merge(gemini_routes(true, false));

    let grouped_compat_routes = Router::new()
        .merge(openai_routes(true, true))
        .merge(claude_routes(true, true))
        .merge(gemini_routes(true, true));

    // 2. Build routers for Ollama chat
    let ollama_normal_chat = ollama_chat_routes(false, false);
    let ollama_grouped_normal_chat = ollama_chat_routes(false, true);
    let ollama_compat_chat = ollama_chat_routes(true, false);
    let ollama_grouped_compat_chat = ollama_chat_routes(true, true);

    // 3. Unauthenticated routes
    let unauthenticated_router = Router::new()
        .route("/", get(|| async { "Chatspeed ccproxy is running." }))
        .route(
            "/favicon.ico",
            get(|| async { (StatusCode::NOT_FOUND, "") }),
        );

    log_registered_routes();

    // 4. Combine all routers into the final application router.
    Router::new()
        .merge(unauthenticated_router)
        // Standalone Ollama API routes (no grouping/compat)
        .merge(ollama_api_routes().layer(ollama_auth_middleware.clone()))
        // --- Route Combination Logic ---
        // Non-grouped, normal mode
        .merge(normal_routes.clone().layer(auth_middleware.clone()))
        .merge(
            ollama_normal_chat
                .clone()
                .layer(ollama_auth_middleware.clone()),
        )
        // Grouped, normal mode
        .nest(
            "/{group_name}",
            grouped_normal_routes.layer(auth_middleware.clone()),
        )
        .nest(
            "/{group_name}",
            ollama_grouped_normal_chat.layer(ollama_auth_middleware.clone()),
        )
        // Non-grouped, compatibility mode
        .nest(
            &format!("/{}", TOOL_COMPAT_MODE_PREFIX),
            compat_routes.clone().layer(auth_middleware.clone()),
        )
        .nest(
            &format!("/{}", TOOL_COMPAT_MODE_PREFIX),
            ollama_compat_chat
                .clone()
                .layer(ollama_auth_middleware.clone()),
        )
        // Grouped, compatibility mode
        .nest(
            &format!("/{{group_name}}/{}", TOOL_COMPAT_MODE_PREFIX),
            grouped_compat_routes.layer(auth_middleware),
        )
        .nest(
            &format!("/{{group_name}}/{}", TOOL_COMPAT_MODE_PREFIX),
            ollama_grouped_compat_chat.layer(ollama_auth_middleware),
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
) -> Result<Response, StatusCode> {
    let path = req.uri().path().to_string();
    match authenticate_request(headers, query, state.main_store.clone(), is_local).await {
        Ok(_) => Ok(next.run(req).await),
        Err(e) => {
            log::warn!("Authentication failed for path {}: {:?}", path, e);
            Err(StatusCode::UNAUTHORIZED)
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
    log::info!("  - /v1/models, /v1/chat/completions,");
    log::info!("[Claude-Compatible]");
    log::info!("  - /v1/messages");
    log::info!("[Gemini-Compatible]");
    log::info!("  - /v1beta/models, /v1beta/models/{{model_id}}:{{action}}");
    log::info!("[Ollama-Specific]");
    log::info!("  - /api/tags, /api/show, /api/chat");
    log::info!("[Access Modes]");
    log::info!("  - Grouped:           /{{group_name}}/{{path}}");
    log::info!("  - Tool Compat:       /compat_mode/{{path}}");
    log::info!("  - Grouped + Compat:  /{{group_name}}/compat_mode/{{path}}");
    log::info!("-------------------------------------");
}
