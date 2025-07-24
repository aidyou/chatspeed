use crate::ccproxy::gemini::GeminiRequest;
use crate::ccproxy::gemini_handler::handle_gemini_native_chat;
use crate::ccproxy::{
    auth::authenticate_request,
    claude_handler::handle_claude_native_chat,
    openai_handler::{handle_chat_completion_proxy, list_proxied_models_handler},
    types::{claude::ClaudeNativeRequest, openai::OpenAIChatCompletionRequest},
};
use crate::db::MainStore;

use serde::Deserialize;
use std::sync::{Arc, Mutex};
use warp::{filters::header::headers_cloned, Filter, Rejection, Reply};

#[derive(Deserialize)]
struct GeminiQuery {
    key: String,
}

/// Helper function to inject MainStore into Warp filters.
fn with_main_store(
    main_store_arc: Arc<Mutex<MainStore>>,
) -> impl Filter<Extract = (Arc<Mutex<MainStore>>,), Error = std::convert::Infallible> + Clone {
    #[cfg(debug_assertions)]
    log::debug!("with_main_store: MainStore injected into filters.");

    warp::any().map(move || main_store_arc.clone())
}

/// Defines all routes for the ccproxy module.
pub fn routes(
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
        .and(warp::body::bytes())
        .and(with_main_store(main_store_arc.clone()))
        .and_then(handle_chat_completion_proxy)
        .map(|reply| (reply,)); // Wrap the Reply in a tuple

    // POST /ccproxy/v1/messages - Claude messages
    let claude_messages_route = warp::path!("claude" / "v1" / "messages")
        .map(|| log::debug!("Matched /claude/v1/messages"))
        .and(warp::post())
        .and(auth_filter.clone())
        .untuple_one()
        .and(headers_cloned())
        .and(warp::body::bytes())
        .and(with_main_store(main_store_arc.clone()))
        .and_then(handle_claude_native_chat)
        .map(|reply| (reply,));

    //
    let gemini_generate_content_route =
        warp::path!("gemini" / "v1beta" / "models" / String / String)
            .and(warp::query::<GeminiQuery>())
            .map(|model_name: String, action: String, key: GeminiQuery| {
                log::debug!(
                    "Matched /gemini/v1beta/models/{}/{}?key=***",
                    model_name,
                    action
                );
                (model_name, action, key.key)
            })
            .and(warp::post())
            .and(auth_filter.clone()) // Reuse auth_filter, ensure it's Clone
            .untuple_one()
            .and(headers_cloned()) // Extract original client headers
            .and(warp::body::bytes())
            .and(with_main_store(main_store_arc.clone()))
            .and_then(handle_gemini_native_chat)
            .map(|reply| (reply,)); // Wrap the Reply in a tuple

    log::info!("Registered GET /ccproxy/v1/models");
    log::info!("Registered POST /ccproxy/v1/chat/completions");
    log::info!("Registered POST /ccproxy/claude/v1/messages");
    log::info!(
        "Registered POST /ccproxy/gemini/v1beta/models/{{modelId}}/generateContent?key=API-Key"
    );
    list_models_route
        .or(chat_completions_route)
        .or(claude_messages_route)
        .or(gemini_generate_content_route)
}
