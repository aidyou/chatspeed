use crate::ai::interaction::ChatProtocol;
use crate::ccproxy::auth::authenticate_request;
use crate::ccproxy::helper::CcproxyQuery;
use crate::ccproxy::{
    handle_ollama_tags, handle_openai_chat_completion, handle_openai_list_models,
};
use crate::db::MainStore;
use std::net::SocketAddr;

use serde_json::json;
use std::sync::{Arc, Mutex};
use warp::{filters::header::headers_cloned, Filter, Rejection, Reply};

const TOOL_COMPAT_MODE: &str = "compat_mode";

/// Helper function to inject MainStore into Warp filters.
fn with_main_store(
    main_store_arc: Arc<Mutex<MainStore>>,
) -> impl Filter<Extract = (Arc<Mutex<MainStore>>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || main_store_arc.clone())
}

/// Filter to check if the remote address is a loopback address.
fn is_loopback() -> impl Filter<Extract = (), Error = Rejection> + Clone {
    warp::addr::remote()
        .and_then(|addr: Option<SocketAddr>| async move {
            match addr {
                Some(addr) if addr.ip().is_loopback() => {
                    log::debug!("{} connected", addr.ip());
                    Ok(())
                }
                _ => Err(warp::reject::custom(
                    crate::ccproxy::errors::CCProxyError::Forbidden(
                        "Access denied: Only local connections are allowed.".to_string(),
                    ),
                )),
            }
        })
        .untuple_one()
}

/// Defines all routes for the ccproxy module.
pub fn routes(
    main_store_arc: Arc<Mutex<MainStore>>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    let ollama_root_route = warp::path::end()
        .and(warp::get())
        .map(|| warp::reply::json(&json!({ "message": "Chatspeed's ccproxy module now supports proxying requests between Ollama, OpenAI-compatible, Gemini, and Claude protocols." })));

    let ollama_version_route = warp::path!("api" / "version")
        .and(warp::get())
        .map(|| warp::reply::json(&json!({ "version": "1.0.0" })));

    let ollama_list_models_route = warp::path!("api" / "tags")
        .and(warp::get())
        .and(with_main_store(main_store_arc.clone()))
        .and_then(|store| handle_ollama_tags(store))
        .map(|reply| (reply,));

    // Ollama routes (IP-based auth)
    let auth_filter_ip = is_loopback();
    let ollama_chat_route = warp::path!("api" / "chat")
        .and(warp::post())
        .and(auth_filter_ip.clone())
        .and(headers_cloned())
        .and(warp::query::<CcproxyQuery>())
        .and(warp::body::bytes())
        .and(with_main_store(main_store_arc.clone()))
        .and_then(|headers, query, body, store| {
            handle_openai_chat_completion(
                ChatProtocol::Ollama,
                headers,
                query,
                body,
                false,
                "".to_string(),
                "".to_string(),
                store,
            )
        })
        .map(|reply| (reply,));

    let auth_filter = headers_cloned()
        .and(warp::query::<CcproxyQuery>())
        .and(with_main_store(main_store_arc.clone()))
        .and_then(authenticate_request)
        .untuple_one();

    let list_models_route = warp::path!("v1" / "models")
        .and(warp::get())
        .and(with_main_store(main_store_arc.clone()))
        .and_then(|store| handle_openai_list_models(store))
        .map(|reply| (reply,));

    // OpenAI routes (API key auth)
    let chat_completions_route = warp::path!("v1" / "chat" / "completions")
        .and(warp::post())
        .and(auth_filter.clone())
        .and(headers_cloned())
        .and(warp::query::<CcproxyQuery>())
        .and(warp::body::bytes())
        .and(with_main_store(main_store_arc.clone()))
        .and_then(|headers, query, body, store| {
            handle_openai_chat_completion(
                ChatProtocol::OpenAI,
                headers,
                query,
                body,
                false,
                "".to_string(),
                "".to_string(),
                store,
            )
        })
        .map(|reply| (reply,));

    // Claude routes (API key auth)
    let claude_messages_route = warp::path!("v1" / "messages")
        .and(warp::post())
        .and(auth_filter.clone())
        .and(headers_cloned())
        .and(warp::query::<CcproxyQuery>())
        .and(warp::body::bytes())
        .and(with_main_store(main_store_arc.clone()))
        .and_then(|headers, query, body, store| {
            handle_openai_chat_completion(
                ChatProtocol::Claude,
                headers,
                query,
                body,
                false,
                "".to_string(),
                "".to_string(),
                store,
            )
        })
        .map(|reply| (reply,));

    // Gemini routes (API key auth)
    let gemini_generate_content_route = warp::path!("v1beta" / "models" / String / ..)
        .and(warp::path::param::<String>()) // action
        .and(warp::post())
        .and(auth_filter.clone())
        .and(headers_cloned())
        .and(warp::query::<CcproxyQuery>())
        .and(warp::body::bytes())
        .and(with_main_store(main_store_arc.clone()))
        .and_then(|model, action, headers, query, body, store| {
            handle_openai_chat_completion(
                ChatProtocol::Gemini,
                headers,
                query,
                body,
                false,
                model,
                action,
                store,
            )
        })
        .map(|reply| (reply,));

    // Tool Compatibility Mode Routes
    let compat_mode_path = warp::path::param::<String>().and_then(|mode: String| async move {
        if mode == TOOL_COMPAT_MODE {
            Ok((true,))
        } else {
            Err(warp::reject::not_found())
        }
    });

    let openai_compat_chat_completions_route = compat_mode_path
        .clone()
        .and(warp::path!("v1" / "chat" / "completions"))
        .and(warp::post())
        .and(auth_filter.clone())
        .and(headers_cloned())
        .and(warp::query::<CcproxyQuery>())
        .and(warp::body::bytes())
        .and(with_main_store(main_store_arc.clone()))
        .and_then(|(compat,), headers, query, body, store| {
            handle_openai_chat_completion(
                ChatProtocol::OpenAI,
                headers,
                query,
                body,
                compat,
                "".to_string(),
                "".to_string(),
                store,
            )
        })
        .map(|reply| (reply,));

    let claude_compat_chat_completions_route = compat_mode_path
        .clone()
        .and(warp::path!("v1" / "messages"))
        .and(warp::post())
        .and(auth_filter.clone())
        .and(headers_cloned())
        .and(warp::query::<CcproxyQuery>())
        .and(warp::body::bytes())
        .and(with_main_store(main_store_arc.clone()))
        .and_then(|(compat,), headers, query, body, store| {
            handle_openai_chat_completion(
                ChatProtocol::Claude,
                headers,
                query,
                body,
                compat,
                "".to_string(),
                "".to_string(),
                store,
            )
        })
        .map(|reply| (reply,));

    let gemini_compat_chat_completions_route = compat_mode_path
        .clone()
        .and(warp::path!("v1beta" / "models" / String / ..))
        .and(warp::path::param::<String>()) // action
        .and(warp::post())
        .and(auth_filter.clone())
        .and(headers_cloned())
        .and(warp::query::<CcproxyQuery>())
        .and(warp::body::bytes())
        .and(with_main_store(main_store_arc.clone()))
        .and_then(|(compat,), model, action, headers, query, body, store| {
            handle_openai_chat_completion(
                ChatProtocol::Gemini,
                headers,
                query,
                body,
                compat,
                model,
                action,
                store,
            )
        })
        .map(|reply| (reply,));

    log::info!("Registered Ollama route: GET /");
    log::info!("Registered Ollama route: GET /api/version");
    log::info!("Registered Ollama route: GET /api/tags");
    log::info!("Registered Ollama route: POST /api/chat");
    log::info!("Registered OpenAI route: POST /v1/chat/completions");
    log::info!("Registered OpenAI route: GET /v1/models");
    log::info!("Registered Claude route: POST /v1/messages");
    log::info!("Registered Gemini route: POST /v1beta/models/...");

    ollama_root_route
        .or(ollama_version_route)
        .or(ollama_chat_route)
        .or(ollama_list_models_route)
        .or(list_models_route)
        .or(chat_completions_route)
        .or(claude_messages_route)
        .or(gemini_generate_content_route)
        .or(openai_compat_chat_completions_route)
        .or(claude_compat_chat_completions_route)
        .or(gemini_compat_chat_completions_route)
        .map(|reply| reply)
}
