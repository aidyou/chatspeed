use crate::{
    ai::{interaction::ChatProtocol, network::ProxyType, util::get_proxy_type},
    ccproxy::{
        errors::{CCProxyError, ProxyResult},
        helper::CC_PROXY_ROTATOR,
        types::{BackendModelTarget, ChatCompletionProxyConfig, ProxyModel},
    },
    commands::chat::setup_chat_proxy,
    db::{AiModel, MainStore},
};
use reqwest::Client;
use serde::Deserialize;
use std::{
    str::FromStr,
    sync::{Arc, Mutex},
    vec,
};

pub const DEFAULT_LOG_TO_FILE: bool = true;

#[derive(Deserialize)]
pub struct CcproxyQuery {
    pub key: Option<String>,
    pub debug: Option<bool>,
}

/// Common logic for model retrieval and rotation
pub struct ModelResolver;

impl ModelResolver {
    /// Retrieves AI model details and API key using global rotation by proxy alias.
    /// This method implements true global key rotation across all providers.
    ///
    /// # Aguments
    ///
    /// * `main_store_arc` - An `Arc` wrapped `Mutex` of the `MainStore` instance.
    /// * `proxy_alias` - The proxy alias to use for model selection.
    ///
    /// # Returns
    ///
    /// * A tuple containing the `AiModel` details
    /// * The selected API key.
    /// * The Actual model id
    /// ```
    pub async fn get_ai_model_by_alias(
        main_store_arc: Arc<Mutex<MainStore>>,
        proxy_alias: String,
    ) -> ProxyResult<ProxyModel> {
        // First, ensure the global key pool is up to date
        Self::update_global_key_pool(&main_store_arc, &proxy_alias).await?;

        // Get the next key from the global pool
        let global_key = CC_PROXY_ROTATOR
            .get_next_global_key(&proxy_alias)
            .await
            .ok_or_else(|| {
                log::warn!(
                    "No keys available in global pool for alias '{}'",
                    proxy_alias
                );
                CCProxyError::NoBackendTargets(proxy_alias.clone())
            })?;

        // Get the AI model details for the selected provider
        let ai_model_details = Self::get_ai_model_details(&main_store_arc, global_key.provider_id)?;

        log::info!(
            "chat_completion_proxy: alias={}, provider={}, base_url={}, protocol={}, selected={}",
            &proxy_alias,
            &ai_model_details.name,
            &ai_model_details.base_url,
            &ai_model_details.api_protocol,
            &global_key.key[..std::cmp::min(8, global_key.key.len())] // Log first 8 chars for debugging
        );

        if ai_model_details.base_url.is_empty() {
            log::error!(
                "Target AI provider base_url is empty for alias '{}'",
                proxy_alias
            );
            return Err(CCProxyError::InternalError(
                "Target base URL is empty".to_string(),
            ));
        }
        let backend_chat_protocol = ChatProtocol::from_str(&ai_model_details.api_protocol)
            .map_err(|e| CCProxyError::InvalidProtocolError(e.to_string()))?;

        Ok(ProxyModel {
            provider: ai_model_details.name.clone(),
            chat_protocol: backend_chat_protocol,
            base_url: ai_model_details.base_url,
            model: global_key.model_name,
            api_key: global_key.key,
            metadata: ai_model_details.metadata.clone(),
        })
    }

    /// Update the global key pool for a proxy alias with all available keys from all providers.
    /// This method collects all keys from all backend targets and creates a global pool.
    async fn update_global_key_pool(
        main_store_arc: &Arc<Mutex<MainStore>>,
        proxy_alias: &str,
    ) -> ProxyResult<()> {
        let backend_targets = Self::get_backend_targets(main_store_arc, proxy_alias)?;
        // get next target index
        let index = CC_PROXY_ROTATOR.get_next_target_index(proxy_alias, backend_targets.len());
        let target = &backend_targets[index];

        // Get AI model details for this target
        let ai_model = Self::get_ai_model_details(main_store_arc, target.id)?;

        // Parse all keys for this provider
        let keys: Vec<String> = if ai_model.api_key.contains('\n') {
            ai_model
                .api_key
                .split('\n')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            let k = ai_model.api_key.trim().to_string();
            if k.is_empty() {
                vec![]
            } else {
                vec![k]
            }
        };

        log::info!(
            "Updated global key pool for alias '{}': {} keys from {} providers",
            proxy_alias,
            keys.len(),
            backend_targets.len()
        );

        CC_PROXY_ROTATOR
            .update_provider_keys_efficient(
                proxy_alias,
                ai_model.id.unwrap_or_default(),
                &ai_model.base_url,
                &target.model,
                keys,
            )
            .await;

        Ok(())
    }

    /// Get the list of backend targets corresponding to the model alias
    fn get_backend_targets(
        main_store_arc: &Arc<Mutex<MainStore>>,
        proxy_alias: &str,
    ) -> ProxyResult<Vec<BackendModelTarget>> {
        let store_guard = main_store_arc.lock().map_err(|e| {
            let err_msg = format!("Failed to lock MainStore: {}", e);
            CCProxyError::StoreLockError(err_msg)
        })?;

        let proxy_config: ChatCompletionProxyConfig =
            store_guard.get_config("chat_completion_proxy", std::collections::HashMap::new());

        proxy_config.get(proxy_alias).cloned().ok_or_else(|| {
            log::warn!("Proxy alias '{}' not found in configuration.", proxy_alias);
            CCProxyError::ModelAliasNotFound(proxy_alias.to_string())
        })
    }

    /// Get AI model details by provider_id
    fn get_ai_model_details(
        main_store_arc: &Arc<Mutex<MainStore>>,
        provider_id: i64,
    ) -> ProxyResult<AiModel> {
        let store_guard = main_store_arc.lock().map_err(|e| {
            let err_msg = format!("Failed to lock MainStore: {}", e);
            CCProxyError::StoreLockError(err_msg)
        })?;

        store_guard
            .config
            .get_ai_model_by_id(provider_id)
            .map_err(|e| {
                CCProxyError::ModelDetailsFetchError(format!(
                    "Failed to get model details: {}",
                    e
                ))
            })
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
            CCProxyError::InternalError(err_msg)
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

pub fn get_provider_full_url(
    protocol: ChatProtocol,
    base_url: &str,
    model_id: &str,
    api_key: &str,
    is_streaming_request: bool,
) -> String {
    match protocol {
        ChatProtocol::OpenAI | ChatProtocol::Ollama => {
            format!("{}/chat/completions", base_url.trim_end_matches('/'))
        }
        ChatProtocol::HuggingFace => base_url
            .split_once("/hf-inference/models")
            .map(|(base, _)| {
                format!(
                    "{}/hf-inference/models/{}/v1/chat/completions",
                    base, model_id
                )
            })
            .unwrap_or_else(|| {
                format!(
                    "https://router.huggingface.co/hf-inference/models/{}/v1/chat/completions",
                    model_id
                )
            }),
        ChatProtocol::Claude => format!("{}/messages", base_url.trim_end_matches('/')),
        ChatProtocol::Gemini => {
            let action = if is_streaming_request {
                "streamGenerateContent"
            } else {
                "generateContent"
            };
            format!(
                "{}/models/{}:{}?alt=sse&key={}",
                base_url.trim_end_matches('/'),
                model_id,
                action,
                api_key,
            )
        }
    }
}
