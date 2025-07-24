use crate::{
    ccproxy::{
        errors::{ProxyAuthError, ProxyResult},
        openai::ChatCompletionProxyKeysConfig,
    },
    db::MainStore,
};

use rust_i18n::t;
use std::sync::{Arc, Mutex};

/// Authenticates the request based on the Authorization Bearer token or x-api-key.
/// Reads `chat_completion_proxy_keys` from `MainStore`.
pub async fn authenticate_request(
    headers: warp::http::header::HeaderMap,
    main_store: Arc<Mutex<MainStore>>,
) -> ProxyResult<()> {
    // Check for x-api-key header first (Claude native format)
    let api_key_header = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok());

    // Check for Authorization Bearer token (OpenAI compatible format)
    let auth_header = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok());

    let token_to_check = match (api_key_header, auth_header) {
        // Prefer x-api-key if both are provided
        (Some(api_key), _) => api_key.to_string(),
        (_, Some(header_val)) if header_val.starts_with("Bearer ") => {
            header_val.trim_start_matches("Bearer ").to_string()
        }
        _ => {
            #[cfg(debug_assertions)]
            log::warn!(
                "Proxy authentication: Missing or invalid Authorization header or x-api-key."
            );

            return Err(warp::reject::custom(ProxyAuthError::MissingToken));
        }
    };

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
        return Err(warp::reject::custom(ProxyAuthError::NoKeysConfigured));
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

        Err(warp::reject::custom(ProxyAuthError::InvalidToken))
    }
}
