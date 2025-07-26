use crate::ccproxy::common::CcproxyQuery;
use crate::ccproxy::gemini_handler::handle_gemini_native_chat;
use crate::ccproxy::{
    auth::authenticate_request,
    claude_handler::handle_claude_native_chat,
    handler::openai_handler::{handle_chat_completion_proxy, list_proxied_models_handler},
};
use crate::db::MainStore;

use std::sync::{Arc, Mutex};
use warp::{filters::header::headers_cloned, Filter, Rejection, Reply};

const TOOL_COMPAT_MODE: &str = "tool_compat_mode";

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
        .and(warp::query::<CcproxyQuery>())
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

    // POST /ccproxy/{normal|tool_compat_mode}/v1/chat/completions
    let chat_completions_route = warp::path!(String / "v1" / "chat" / "completions")
        .map(|style: String| {
            log::debug!("Matched /{style}/v1/chat/completions");
            (style == TOOL_COMPAT_MODE.to_string(),)
        })
        .and(warp::post())
        .and(auth_filter.clone()) // Reuse auth_filter, ensure it's Clone
        .untuple_one()
        .and(headers_cloned()) // Extract original client headers
        .and(warp::query::<CcproxyQuery>())
        .and(warp::body::bytes())
        .and(with_main_store(main_store_arc.clone()))
        .and_then(|tool_compat_mode, headers, query, body, store| {
            handle_chat_completion_proxy(headers, query, body, tool_compat_mode, store)
        })
        .map(|reply| (reply,)); // Wrap the Reply in a tuple

    // POST /ccproxy/v1/messages - Claude messages
    let claude_messages_route = warp::path!(String / "claude" / "v1" / "messages")
        .map(|style: String| {
            log::debug!("Matched /{style}/claude/v1/messages");
            (style == TOOL_COMPAT_MODE.to_string(),)
        })
        .and(warp::post())
        .and(auth_filter.clone())
        .untuple_one()
        .and(headers_cloned())
        .and(warp::query::<CcproxyQuery>())
        .and(warp::body::bytes())
        .and(with_main_store(main_store_arc.clone()))
        .and_then(|tool_compat_mode, headers, query, body, store| {
            handle_claude_native_chat(headers, query, body, tool_compat_mode, store)
        })
        .map(|reply| (reply,));

    //
    let gemini_generate_content_route =
        warp::path!(String / "gemini" / "v1beta" / "models" / String / String)
            .and(warp::query::<CcproxyQuery>())
            .map(
                |style: String, model_name: String, action: String, query: CcproxyQuery| {
                    log::debug!(
                        "Matched /gemini/v1beta/models/{}/{}?key=***",
                        model_name,
                        action
                    );
                    (
                        style == TOOL_COMPAT_MODE.to_string(),
                        model_name,
                        action,
                        query,
                    )
                },
            )
            .and(warp::post())
            .and(auth_filter.clone()) // Reuse auth_filter, ensure it's Clone
            .untuple_one()
            .and(headers_cloned()) // Extract original client headers
            .and(warp::body::bytes())
            .and(with_main_store(main_store_arc.clone()))
            .and_then(
                |tool_compat_mode, model_name, action, query, headers, body, store| {
                    handle_gemini_native_chat(
                        model_name,
                        action,
                        query,
                        headers,
                        body,
                        tool_compat_mode,
                        store,
                    )
                },
            )
            .map(|reply| (reply,)); // Wrap the Reply in a tuple

    log::info!("Registered GET /ccproxy/v1/models");
    log::info!("Registered POST /ccproxy/tool_use/v1/chat/completions");
    log::info!("Registered POST /ccproxy/tool_compat_mode/v1/chat/completions");
    log::info!("Registered POST /ccproxy/tool_use/claude/v1/messages");
    log::info!("Registered POST /ccproxy/tool_compat_mode/claude/v1/messages");
    log::info!(
        "Registered POST /ccproxy/tool_use/gemini/v1beta/models/{{modelId}}/generateContent?key=API-Key"
    );
    log::info!(
        "Registered POST /ccproxy/tool_compat_mode/gemini/v1beta/models/{{modelId}}/generateContent?key=API-Key"
    );
    list_models_route
        .or(chat_completions_route)
        .or(claude_messages_route)
        .or(gemini_generate_content_route)
}
