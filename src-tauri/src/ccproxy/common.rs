use crate::{
    ai::{network::ProxyType, util::get_proxy_type},
    ccproxy::{
        errors::{ProxyAuthError, ProxyResult},
        openai::ChatCompletionProxyConfig,
        proxy_rotator::PROXY_BACKEND_ROTATOR,
        types::openai::BackendModelTarget,
    },
    commands::chat::setup_chat_proxy,
    db::{AiModel, MainStore},
};
use reqwest::Client;
use serde::Deserialize;
use std::sync::{Arc, Mutex};

pub const DEFAULT_LOG_TO_FILE: bool = true;

#[derive(Deserialize)]
pub struct CcproxyQuery {
    pub key: Option<String>,
    pub debug: Option<bool>,
}

/// Common logic for model retrieval and rotation
pub struct ModelResolver;

impl ModelResolver {
    /// Retrieves AI model details and selected backend target by proxy alias
    pub async fn get_ai_model_by_alias(
        main_store_arc: Arc<Mutex<MainStore>>,
        proxy_alias: String,
    ) -> ProxyResult<(AiModel, BackendModelTarget)> {
        let backend_targets = Self::get_backend_targets(&main_store_arc, &proxy_alias)?;

        if backend_targets.is_empty() {
            log::warn!(
                "Proxy alias '{}' has no backend targets configured.",
                proxy_alias
            );
            return Err(warp::reject::custom(ProxyAuthError::NoBackendTargets(
                proxy_alias,
            )));
        }

        // Select a backend target using the rotation mechanism
        let selected_target = Self::rotator_models(&proxy_alias, backend_targets).await;

        // Get AI model details
        let ai_model_details = Self::get_ai_model_details(&main_store_arc, selected_target.id)?;

        log::info!(
            "chat_completion_proxy: alias={}, provider={}, base_url={}, protocol={}",
            &proxy_alias,
            &ai_model_details.name,
            &ai_model_details.base_url,
            &ai_model_details.api_protocol
        );

        if ai_model_details.base_url.is_empty() {
            log::error!(
                "Target AI provider base_url is empty for alias '{}'",
                proxy_alias
            );
            return Err(warp::reject::custom(ProxyAuthError::InternalError(
                "Target base URL is empty".to_string(),
            )));
        }

        Ok((ai_model_details, selected_target))
    }

    /// Get the list of backend targets corresponding to the model alias
    fn get_backend_targets(
        main_store_arc: &Arc<Mutex<MainStore>>,
        proxy_alias: &str,
    ) -> ProxyResult<Vec<BackendModelTarget>> {
        let store_guard = main_store_arc.lock().map_err(|e| {
            let err_msg = format!("Failed to lock MainStore: {}", e);
            warp::reject::custom(ProxyAuthError::StoreLockError(err_msg))
        })?;

        let proxy_config: ChatCompletionProxyConfig =
            store_guard.get_config("chat_completion_proxy", std::collections::HashMap::new());

        proxy_config.get(proxy_alias).cloned().ok_or_else(|| {
            log::warn!("Proxy alias '{}' not found in configuration.", proxy_alias);
            warp::reject::custom(ProxyAuthError::ModelAliasNotFound(proxy_alias.to_string()))
        })
    }

    /// Select a backend target using the rotation mechanism
    pub async fn rotator_models(
        proxy_alias: &str,
        backend_targets: Vec<BackendModelTarget>,
    ) -> BackendModelTarget {
        let selected_index = PROXY_BACKEND_ROTATOR
            .get_next_index(proxy_alias, backend_targets.len())
            .await;
        backend_targets[selected_index].clone()
    }

    /// Get AI model details by provider_id
    fn get_ai_model_details(
        main_store_arc: &Arc<Mutex<MainStore>>,
        provider_id: i64,
    ) -> ProxyResult<AiModel> {
        let store_guard = main_store_arc.lock().map_err(|e| {
            let err_msg = format!("Failed to lock MainStore: {}", e);
            warp::reject::custom(ProxyAuthError::StoreLockError(err_msg))
        })?;

        store_guard
            .config
            .get_ai_model_by_id(provider_id)
            .map_err(|e| {
                warp::reject::custom(ProxyAuthError::ModelDetailsFetchError(format!(
                    "Failed to get model details: {}",
                    e
                )))
            })
    }

    /// Rotate API keys (supports multiple key configurations)
    /// TODO: 相同 url 下，不同 key 混淆情况
    pub async fn rotator_keys(base_url: &str, target_api_key: String) -> String {
        if target_api_key.contains('\n') {
            let keys = target_api_key
                .split('\n')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<String>>();
            crate::ai::interaction::constants::API_KEY_ROTATOR
                .get_next_key(&base_url, keys)
                .await
                .unwrap_or_default()
        } else {
            target_api_key
        }
    }

    pub async fn build_http_client(
        main_store_arc: Arc<Mutex<MainStore>>,
        mut metadata: Option<serde_json::Value>,
    ) -> ProxyResult<Client> {
        let mut client_builder = Client::builder();
        let _ = setup_chat_proxy(&main_store_arc, &mut metadata);
        let proxy_type = get_proxy_type(metadata);

        match proxy_type {
            ProxyType::None => {
                client_builder = client_builder.no_proxy();
            }
            ProxyType::Http(proxy_url, proxy_username, proxy_password) => {
                log::info!("Using proxy: {}", proxy_url);
                if let Ok(mut proxy) = reqwest::Proxy::all(&proxy_url) {
                    if let (Some(user), Some(pass)) = (proxy_username, proxy_password) {
                        if !user.is_empty() && !pass.is_empty() {
                            proxy = proxy.basic_auth(&user, &pass);
                        }
                    }
                    client_builder = client_builder.proxy(proxy);
                }
            }
            ProxyType::System => { /* Use system proxy settings by default */ }
        }

        client_builder.build().map_err(|e| {
            let err_msg = format!("Failed to build HTTP client: {}", e);
            log::error!("{}", err_msg);
            warp::reject::custom(ProxyAuthError::InternalError(err_msg))
        })
    }
}

/// Determines whether a given header should be forwarded
///
/// # Arguments
/// * `name_str` - The name of the header to check
///
/// # Returns
/// * `true` if the header should be forwarded
/// * `false` if the header should not be forwarded
///
/// Currently allows forwarding of:
/// - user-agent
/// - accept-language
/// - http-referer
/// - Any header starting with "x-" (custom headers)
pub fn should_forward_header(name_str: &str) -> bool {
    return name_str == "x-request-id"
            || name_str == "user-agent"
            || name_str == "accept-language"
            || name_str == "http-referer"
            // Add other custom headers you want to forward, e.g., "x-custom-tenant-id"
            || (name_str.starts_with("x-") && !name_str.starts_with("x-api-"));
}
