use crate::ai::interaction::{chat_completion::ChatState, ChatProtocol};
use crate::ccproxy::{
    auth::authenticate_request,
    handle_ollama_tags, handle_openai_chat_completion, handle_openai_list_models,
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

const TOOL_COMPAT_MODE: &str = "compat_mode";

/// Defines all routes for the ccproxy module.
pub async fn routes(
    app_handle: tauri::AppHandle,
    main_store_arc: Arc<Mutex<MainStore>>,
    chat_state: Arc<ChatState>, //keep this
) -> Router {
    let shared_state = Arc::new(SharedState {
        app_handle,
        main_store: main_store_arc,
        chat_state,
    });

    // Router for unauthenticated routes (e.g., root path)
    let unauthenticated_router = Router::new()
        .route(
            "/",
            get(|| async {
                "Chatspeed's ccproxy module now supports proxying requests between Ollama, OpenAI-compatible, Gemini, and Claude protocols."
            }),
        )
        .route(
            "/favicon.ico",
            get(|| async {
                // Return 404 for favicon requests to avoid authentication warnings
                (StatusCode::NOT_FOUND, "")
            }),
        );

    // Router for Ollama routes with conditional authentication
    let ollama_routes_inner = Router::new()
        .route(
            "/api/version",
            get(|State(state): State<Arc<SharedState>>| async move {
                let version = state.app_handle.package_info().version.to_string();
                (StatusCode::OK, axum::Json(json!({ "version": version }))).into_response()
            }),
        )
        .route(
            "/api/tags",
            get(|State(state): State<Arc<SharedState>>| async move {
                handle_ollama_tags(state.main_store.clone())
                    .await
                    .into_response()
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
        .route(
            "/api/chat",
            post(
                |State(state): State<Arc<SharedState>>,
                 headers: HeaderMap,
                 Query(query): Query<CcproxyQuery>,
                 body: Bytes| async move {
                    handle_openai_chat_completion(
                        ChatProtocol::Ollama,
                        headers,
                        query,
                        body,
                        false,
                        "".to_string(),
                        "".to_string(),
                        state.main_store.clone(),
                    )
                    .await
                    .into_response()
                },
            ),
        );

    // Middleware for Ollama routes: check if local, otherwise authenticate
    let ollama_auth_middleware = middleware::from_fn_with_state(
        shared_state.clone(),
        |State(state): State<Arc<SharedState>>,
         ConnectInfo(addr): ConnectInfo<SocketAddr>,
         req: http::Request<axum::body::Body>,
         next: Next| async move {
            // Check if the request is from a local address
            let is_local = addr.ip().is_loopback();

            if is_local {
                #[cfg(debug_assertions)]
                log::debug!(
                    "Ollama local access: {} connected, bypassing authentication",
                    addr.ip()
                );
                let response = next.run(req).await;
                #[cfg(debug_assertions)]
                log::debug!("Ollama local access: request completed successfully");
                return Ok(response);
            }

            // If not local, then authenticate
            #[cfg(debug_assertions)]
            log::debug!(
                "Ollama non-local access: {} requires authentication",
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
                is_local, // Pass is_local to the authentication middleware
            )
            .await
        },
    );

    let ollama_router = ollama_routes_inner.layer(ollama_auth_middleware);

    // Create reusable authentication middleware
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

    // Router for authenticated routes (OpenAI, Claude, Gemini)
    let authenticated_router = Router::new()
        .route(
            "/v1/models",
            get(|State(state): State<Arc<SharedState>>| async move {
                handle_openai_list_models(state.main_store.clone())
                    .await
                    .into_response()
            }),
        )
        .route(
            "/v1beta/models",
            get(|State(state): State<Arc<SharedState>>| async move {
                handle_gemini_list_models(state.main_store.clone())
                    .await
                    .into_response()
            }),
        )
        .route(
            "/v1/chat/completions",
            post(
                |State(state): State<Arc<SharedState>>,
                 headers: HeaderMap,
                 Query(query): Query<CcproxyQuery>,
                 body: Bytes| async move {
                    handle_openai_chat_completion(
                        ChatProtocol::OpenAI,
                        headers,
                        query,
                        body,
                        false,
                        "".to_string(),
                        "".to_string(),
                        state.main_store.clone(),
                    )
                    .await
                    .into_response()
                },
            ),
        )
        .route(
            "/v1/messages",
            post(
                |State(state): State<Arc<SharedState>>,
                 headers: HeaderMap,
                 Query(query): Query<CcproxyQuery>,
                 body: Bytes| async move {
                    handle_openai_chat_completion(
                        ChatProtocol::Claude,
                        headers,
                        query,
                        body,
                        false,
                        "".to_string(),
                        "".to_string(),
                        state.main_store.clone(),
                    )
                    .await
                    .into_response()
                },
            ),
        )
        .route(
            "/v1beta/models/{model_id}/{action}",
            post(
                |State(state): State<Arc<SharedState>>,
                 Path((model_id, action)): Path<(String, String)>,
                 headers: HeaderMap,
                 Query(query): Query<CcproxyQuery>,
                 body: Bytes| async move {
                    handle_openai_chat_completion(
                        ChatProtocol::Gemini,
                        headers,
                        query,
                        body,
                        false,
                        model_id,
                        action,
                        state.main_store.clone(),
                    )
                    .await
                    .into_response()
                },
            ),
        )
        .layer(auth_middleware.clone());

    // Compatibility mode routes (always authenticated)
    let compat_mode_routes = Router::new()
        .route(
            "/v1/chat/completions",
            post(
                |State(state): State<Arc<SharedState>>,
                 headers: HeaderMap,
                 Query(query): Query<CcproxyQuery>,
                 body: Bytes| async move {
                    handle_openai_chat_completion(
                        ChatProtocol::OpenAI,
                        headers,
                        query,
                        body,
                        true,
                        "".to_string(),
                        "".to_string(),
                        state.main_store.clone(),
                    )
                    .await
                    .into_response()
                },
            ),
        )
        .route(
            "/v1/messages",
            post(
                |State(state): State<Arc<SharedState>>,
                 headers: HeaderMap,
                 Query(query): Query<CcproxyQuery>,
                 body: Bytes| async move {
                    handle_openai_chat_completion(
                        ChatProtocol::Claude,
                        headers,
                        query,
                        body,
                        true,
                        "".to_string(),
                        "".to_string(),
                        state.main_store.clone(),
                    )
                    .await
                    .into_response()
                },
            ),
        )
        .route(
            "/v1beta/models/{model_id}/{action}",
            post(
                |State(state): State<Arc<SharedState>>,
                 Path((model_id, action)): Path<(String, String)>,
                 headers: HeaderMap,
                 Query(query): Query<CcproxyQuery>,
                 body: Bytes| async move {
                    handle_openai_chat_completion(
                        ChatProtocol::Gemini,
                        headers,
                        query,
                        body,
                        true,
                        model_id,
                        action,
                        state.main_store.clone(),
                    )
                    .await
                    .into_response()
                },
            ),
        )
        .layer(auth_middleware);

    log::info!("Registered Ollama route: GET /");
    log::info!("Registered Ollama route: GET /api/version");
    log::info!("Registered Ollama route: POST /api/chat");
    log::info!("Registered Ollama route: GET /api/tags");
    log::info!("Registered Ollama route: POST /api/show");
    log::info!("Registered OpenAI route: POST /v1/chat/completions");
    log::info!("Registered OpenAI route: GET /v1/models");
    log::info!("Registered Claude route: POST /v1/messages");
    log::info!("Registered Gemini route: POST /v1beta/models/...");
    log::info!("Registered Gemini route: GET /v1beta/models");

    // MCP will be available on a separate port for now due to router state type incompatibility
    // TODO: Future improvement could integrate MCP into ccproxy with proper state handling
    log::info!("MCP service will be available on a separate port with endpoints /sse and /message");

    Router::new()
        .merge(unauthenticated_router) // Merge unauthenticated routes
        .merge(ollama_router) // Merge Ollama routes at root level
        .merge(authenticated_router) // Merge other authenticated routes
        .nest(&format!("/{}", TOOL_COMPAT_MODE), compat_mode_routes) // Nest compatibility mode routes
        .with_state(shared_state)
}

// A struct to hold the shared state
pub struct SharedState {
    pub app_handle: tauri::AppHandle,
    pub main_store: Arc<Mutex<MainStore>>,
    pub chat_state: Arc<ChatState>,
}

// Middleware for authentication
async fn authenticate_request_middleware(
    State(state): State<Arc<SharedState>>,
    headers: HeaderMap,
    Query(query): Query<CcproxyQuery>,
    req: http::Request<axum::body::Body>,
    next: Next,
    is_local: bool, // Add is_local parameter
) -> Result<Response, StatusCode> {
    #[cfg(debug_assertions)]
    log::debug!(
        "authenticate_request_middleware called for path: {}, is_local: {}",
        req.uri().path(),
        is_local
    );

    match authenticate_request(headers, query, state.main_store.clone(), is_local).await {
        Ok(_) => Ok(next.run(req).await),
        Err(e) => {
            log::warn!(
                "Authentication failed for path {}: {:?}",
                req.uri().path(),
                e
            );
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}
