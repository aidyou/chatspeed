use crate::{
    ccproxy::{
        errors::{CCProxyError, ProxyResult},
        helper::CcproxyQuery,
        types::ChatCompletionProxyKeysConfig,
    },
    db::MainStore,
};

use rust_i18n::t;
use std::sync::{Arc, Mutex};

/// Authenticates the request based on the Authorization Bearer token or x-api-key.
/// Reads `chat_completion_proxy_keys` from `MainStore`.
pub async fn authenticate_request(
    headers: warp::http::header::HeaderMap,
    query: CcproxyQuery,
    main_store: Arc<Mutex<MainStore>>,
) -> ProxyResult<()> {
    // In order of priority, we check for an API key in:
    // 1. "Authorization: Bearer <token>" header (OpenAI format)
    // 2. "x-api-key: <token>" header (Claude format)
    // 3. "key=<token>" query parameter (Google AI Studio format)
    // The first non-empty key found is used.

    let bearer_token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let api_key_header = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let query_param_key = query
        .key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let token_to_check = bearer_token
        .or(api_key_header)
        .or(query_param_key)
        .map(str::to_string)
        .ok_or_else(|| {
            #[cfg(debug_assertions)]
            log::warn!(
                "Proxy authentication: Missing token from 'Authorization' header, 'x-api-key' header, or 'key' query param."
            );
            warp::reject::custom(CCProxyError::MissingToken)
        })?;

    if token_to_check.is_empty() {
        log::warn!("Proxy authentication: Bearer token, x-api-key header, or query parameter `key` is missing or empty.");
        return Err(warp::reject::custom(CCProxyError::MissingToken));
    }

    let proxy_keys: ChatCompletionProxyKeysConfig = {
        if let Ok(store_guard) = main_store.lock() {
            // Get the raw string value for chat_completion_proxy_keys
            let keys_config: ChatCompletionProxyKeysConfig =
                store_guard.get_config("chat_completion_proxy_keys", vec![]);
            drop(store_guard);

            keys_config
        } else {
            log::error!("{}", t!("db.failed_to_lock_main_store").to_string());
            vec![]
        }
    };

    if proxy_keys.is_empty() {
        log::warn!("Proxy authentication: 'chat_completion_proxy_keys' is configured but the list is empty.");
        return Err(warp::reject::custom(CCProxyError::NoKeysConfigured));
    }

    if proxy_keys.iter().any(|k| k.token == token_to_check) {
        #[cfg(debug_assertions)]
        log::debug!("Proxy authentication: Token is valid.");

        Ok(())
    } else {
        #[cfg(debug_assertions)]
        log::debug!(
            "Proxy authentication: Token is invalid. Token: {:?}******",
            token_to_check.get(..5).unwrap_or(&token_to_check)
        );

        Err(warp::reject::custom(CCProxyError::InvalidToken))
    }
}
