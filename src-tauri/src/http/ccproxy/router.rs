use crate::db::MainStore;
use crate::http::ccproxy::{
    claude_handler::handle_claude_native_chat,
    claude_types::ClaudeNativeRequest,
    openai_handler::{
        authenticate_request, handle_chat_completion_proxy, list_proxied_models_handler,
    },
    openai_types::OpenAIChatCompletionRequest,
};

use std::sync::{Arc, Mutex};
use warp::{filters::header::headers_cloned, Filter, Rejection, Reply};

/// Helper function to inject MainStore into Warp filters.
fn with_main_store(
    main_store_arc: Arc<Mutex<MainStore>>,
) -> impl Filter<Extract = (Arc<Mutex<MainStore>>,), Error = std::convert::Infallible> + Clone {
    #[cfg(debug_assertions)]
    log::debug!("with_main_store: MainStore injected into filters.");

    warp::any().map(move || main_store_arc.clone())
}

/// Defines all routes for the ccproxy module.
pub fn ccproxy_routes(
    main_store_arc: Arc<Mutex<MainStore>>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    // MODIFIED: Extract type is now a tuple
    // Authentication filter chain
    let auth_filter = headers_cloned()
        .and(with_main_store(main_store_arc.clone()))
        .and_then(authenticate_request)
        .untuple_one();

    // GET /ccproxy/v1/models
    let list_models_route = warp::path!("v1" / "models") // Define path relative to the base in server.rs
        .map(|| log::debug!("Matched /v1/models")) // Optional: log when path is matched
        .and(warp::get())
        .and(auth_filter.clone())
        .untuple_one()
        .and(with_main_store(main_store_arc.clone()))
        .and_then(list_proxied_models_handler)
        .map(|reply| (reply,)); // ADDED: Wrap the Reply in a tuple

    // POST /ccproxy/v1/chat/completions
    let chat_completions_route = warp::path!("v1" / "chat" / "completions")
        .map(|| log::debug!("Matched /v1/chat/completions"))
        .and(warp::post())
        .and(auth_filter.clone()) // Reuse auth_filter, ensure it's Clone
        .untuple_one()
        .and(headers_cloned()) // Extract original client headers
        .and(warp::body::json::<OpenAIChatCompletionRequest>())
        .and(with_main_store(main_store_arc.clone()))
        .and_then(handle_chat_completion_proxy)
        .map(|reply| (reply,)); // Wrap the Reply in a tuple

    // POST /ccproxy/v1/messages - Claude messages
    let claude_messages_route = warp::path!("v1" / "messages")
        .map(|| log::debug!("Matched /v1/messages"))
        .and(warp::post())
        .and(auth_filter.clone())
        .untuple_one()
        .and(headers_cloned())
        .and(warp::body::json::<ClaudeNativeRequest>())
        // .and(warp::body::bytes())
        // .and_then(
        //     |headers: warp::http::HeaderMap, body: bytes::Bytes| async move {
        //         #[cfg(debug_assertions)]
        //         {
        //             if let Ok(body_str) = std::str::from_utf8(&body) {
        //                 log::debug!("Raw ClaudeNativeRequest body: {}", body_str);
        //             } else {
        //                 log::debug!("Raw ClaudeNativeRequest body: <Invalid UTF-8>");
        //             }
        //         }
        //         match serde_json::from_slice::<ClaudeNativeRequest>(&body) {
        //             Ok(req) => Ok((headers, req)), // Keep headers
        //             Err(e) => Err(warp::reject::custom(
        //                 crate::http::ccproxy::errors::ProxyAuthError::InternalError(format!(
        //                     "Failed to deserialize ClaudeNativeRequest: {}",
        //                     e
        //                 )),
        //             )),
        //         }
        //     },
        // )
        // .untuple_one()
        .and(with_main_store(main_store_arc.clone()))
        .and_then(handle_claude_native_chat)
        .map(|reply| (reply,));

    log::info!("Registered GET /ccproxy/v1/models");
    log::info!("Registered POST /ccproxy/v1/chat/completions");
    log::info!("Registered POST /ccproxy/v1/messages");
    list_models_route
        .or(chat_completions_route)
        .or(claude_messages_route)
}
