use crate::{
    ai::interaction::chat_completion::ChatState,
    ccproxy::{
        errors::{CCProxyError, ProxyResult},
        helper::CcproxyQuery,
        types::ChatCompletionProxyKeysConfig,
    },
    constants::INTERNAL_CCPROXY_API_KEY,
    db::MainStore,
};

use http::HeaderMap;
use std::sync::Arc;

/// Authenticates the request based on the Authorization Bearer token or x-api-key.
/// Reads `chat_completion_proxy_keys` from `MainStore`.
pub async fn authenticate_request(
    headers: HeaderMap,
    query: CcproxyQuery,
    main_store: Arc<std::sync::RwLock<MainStore>>,
    chat_state: Arc<ChatState>,
    is_local: bool,
) -> ProxyResult<()> {
    if is_local {
        log::debug!("Skipping authentication for local request.");
        return Ok(());
    }

    // Check for internal request header
    if let Some(internal_header) = headers.get("X-CS-Internal-Request") {
        if internal_header == "true" {
            if let Some(auth_header) = headers.get("authorization") {
                if let Ok(auth_str) = auth_header.to_str() {
                    if let Some(token) = auth_str.strip_prefix("Bearer ") {
                        let internal_key = INTERNAL_CCPROXY_API_KEY.read().clone();
                        if token.trim() == internal_key {
                            log::debug!("Internal request authenticated successfully.");
                            return Ok(());
                        }
                    }
                }
            }
            log::warn!("Internal request authentication failed.");
            return Err(CCProxyError::InvalidToken);
        }
    }

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
            CCProxyError::MissingToken
        })?;

    if token_to_check.is_empty() {
        log::warn!("Proxy authentication: Bearer token, x-api-key header, or query parameter `key` is missing or empty.");
        return Err(CCProxyError::MissingToken);
    }

    // Check if it's a temporary workflow session key
    if let Some(workflow_id) = headers.get("x-cs-workflow-id").and_then(|v| v.to_str().ok()) {
        if let Some(stored_key) = chat_state.workflow_keys.get(workflow_id) {
            if token_to_check == *stored_key {
                log::debug!("Workflow session authenticated successfully.");
                return Ok(());
            }
        }
    }

    let proxy_keys: ChatCompletionProxyKeysConfig = main_store
        .read()
        .map_err(|e| CCProxyError::StoreLockError(e.to_string()))?
        .get_config("chat_completion_proxy_keys", vec![]);

    if proxy_keys.is_empty() {
        log::warn!("Proxy authentication: 'chat_completion_proxy_keys' is configured but the list is empty.");
        return Err(CCProxyError::NoKeysConfigured);
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

        Err(CCProxyError::InvalidToken)
    }
}
