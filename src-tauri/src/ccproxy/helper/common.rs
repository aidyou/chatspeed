use crate::{
    ai::{network::ProxyType, util::get_proxy_type},
    ccproxy::{
        errors::{CCProxyError, ProxyResult},
        helper::proxy_rotator::GlobalApiKey,
        helper::CC_PROXY_ROTATOR,
        types::{BackendModelTarget, ChatCompletionProxyConfig, ProxyModel},
        ChatProtocol,
    },
    commands::chat::setup_chat_proxy,
    constants::CFG_CHAT_COMPLETION_PROXY,
    db::{AiModel, MainStore, ProxyGroup},
};
use reqwest::Client;
use rust_i18n::t;
use serde::Deserialize;
use std::{collections::HashMap, str::FromStr, sync::Arc, vec};

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
        main_store_arc: Arc<std::sync::RwLock<MainStore>>,
        proxy_alias: String,
        proxy_group: Option<&str>,
    ) -> ProxyResult<ProxyModel> {
        // First, ensure the global key pool is up to date
        let (backend_target, ai_model_detail) =
            Self::update_global_key_pool(main_store_arc.clone(), &proxy_alias, proxy_group).await?;

        let group_name = proxy_group.unwrap_or("default");
        let group_config = Self::get_proxy_group(main_store_arc.clone(), group_name);

        let should_enable_injection = group_config
            .as_ref()
            .map(|g| {
                g.metadata
                    .as_ref()
                    .and_then(|m| m.get("modelInjectionCondition"))
                    .and_then(|v| v.as_str())
                    .map(|v| {
                        if v.trim().is_empty() {
                            true
                        } else {
                            v.lines()
                                .map(|line| line.trim())
                                .filter(|line| !line.is_empty())
                                .any(|pattern| {
                                    wildmatch::WildMatch::new(pattern).matches(&proxy_alias)
                                })
                        }
                    })
                    .unwrap_or(true)
            })
            .unwrap_or(false);

        let prompt_injection = if should_enable_injection {
            group_config
                .as_ref()
                .map_or("off".to_string(), |g| g.prompt_injection.clone())
        } else {
            "off".to_string()
        };

        #[cfg(debug_assertions)]
        {
            log::debug!(
                "proxy group: {}, prompt injection: {}",
                group_name,
                &prompt_injection
            );
        }

        let prompt_text = if should_enable_injection {
            group_config
                .as_ref()
                .map(|g| {
                    format!(
                        "<cs:behavioral-guidelines>{}</cs:behavioral-guidelines>",
                        g.prompt_text.clone()
                    )
                })
                .unwrap_or_default()
        } else {
            String::new()
        };

        let prompt_injection_position = group_config
            .as_ref()
            .map(|g| {
                g.metadata
                    .as_ref()
                    .and_then(|m| m.get("promptInjectionPosition"))
                    .and_then(|v| v.as_str())
                    .map(|x| x.to_string())
                    .unwrap_or("system".to_string())
            })
            .unwrap_or("system".to_string());

        let tool_filter = group_config.as_ref().map_or(HashMap::new(), |g| {
            let mut map = HashMap::new();
            for it in g.tool_filter.split('\n').into_iter() {
                if it.trim().is_empty() {
                    continue;
                }
                map.insert(it.trim().to_string(), 1_i8);
            }
            map
        });
        let temperature = group_config
            .as_ref()
            .map_or(1.0, |g| g.temperature.unwrap_or(1.0));

        let prompt_replace = group_config.as_ref().map_or(Vec::new(), |g| {
            g.metadata
                .as_ref()
                .and_then(|m| m.get("promptReplace"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| {
                            let key = item.get("key").and_then(|v| v.as_str())?;
                            let value = item.get("value").and_then(|v| v.as_str())?;
                            if key.is_empty() {
                                None
                            } else {
                                Some((key.to_string(), value.to_string()))
                            }
                        })
                        .collect()
                })
                .unwrap_or_default()
        });

        // Ollama hasn't api key
        if ai_model_detail.api_protocol == ChatProtocol::Ollama.to_string() {
            let custom_params = ai_model_detail
                .models
                .iter()
                .find(|m| m.id == backend_target.model)
                .and_then(|m| m.custom_params.clone());

            return Ok(ProxyModel {
                provider: ai_model_detail.name.clone(),
                chat_protocol: ai_model_detail.api_protocol.try_into().unwrap_or_default(),
                base_url: ai_model_detail.base_url,
                model: backend_target.model.clone(),
                api_key: ai_model_detail.api_key.clone(),
                model_metadata: ai_model_detail.metadata.clone(),
                custom_params,
                prompt_injection_position: Some(prompt_injection_position),
                prompt_injection: prompt_injection,
                prompt_text: prompt_text,
                tool_filter: tool_filter,
                prompt_replace,
                temperature: temperature,
                max_tokens: ai_model_detail.max_tokens,
                presence_penalty: ai_model_detail
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("presencePenalty"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as f32,
                frequency_penalty: ai_model_detail
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("frequencyPenalty"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as f32,
                top_p: ai_model_detail.top_p,
                top_k: ai_model_detail.top_k,
                stop: ai_model_detail
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("stop"))
                    .and_then(|v| v.as_str())
                    .map(|s| {
                        s.lines()
                            .map(|l| l.trim().to_string())
                            .filter(|l| !l.is_empty())
                            .collect()
                    })
                    .unwrap_or_default(),
            });
        }

        let composite_key = format!("{}/{}", group_name, proxy_alias);

        // Get the next key from the global pool
        let global_key = CC_PROXY_ROTATOR
            .get_next_global_key(&composite_key)
            .await
            .ok_or_else(|| {
                log::warn!(
                    "No keys available in global pool for composite key '{}'",
                    composite_key
                );
                CCProxyError::NoBackendTargets(proxy_alias.clone())
            })?;

        // Get the AI model details for the selected provider
        let ai_model_details =
            Self::get_ai_model_details(main_store_arc.clone(), global_key.provider_id)?;

        log::info!(
            "chat_completion_proxy: alias={}, provider={}, base_url={}, protocol={}, selected={}",
            &proxy_alias,
            &ai_model_details.name,
            &ai_model_details.base_url,
            &ai_model_details.api_protocol,
            &global_key.key[std::cmp::max(0, global_key.key.len() - 8)..] // Log last 8 chars for debugging
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

        let custom_params = ai_model_details
            .models
            .iter()
            .find(|m| m.id == global_key.model_name)
            .and_then(|m| m.custom_params.clone());

        Ok(ProxyModel {
            provider: ai_model_details.name.clone(),
            chat_protocol: backend_chat_protocol,
            base_url: ai_model_details.base_url,
            model: global_key.model_name,
            api_key: global_key.key,
            model_metadata: ai_model_details.metadata.clone(),
            custom_params,
            prompt_injection,
            prompt_injection_position: Some(prompt_injection_position),
            prompt_text,
            tool_filter,
            temperature,
            max_tokens: ai_model_details.max_tokens,
            presence_penalty: ai_model_details
                .metadata
                .as_ref()
                .and_then(|m| m.get("presencePenalty"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0) as f32,
            frequency_penalty: ai_model_details
                .metadata
                .as_ref()
                .and_then(|m| m.get("frequencyPenalty"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0) as f32,
            top_p: ai_model_details.top_p,
            top_k: ai_model_details.top_k,
            prompt_replace,
            stop: ai_model_details
                .metadata
                .as_ref()
                .and_then(|m| m.get("stop"))
                .and_then(|v| v.as_str())
                .map(|s| {
                    s.lines()
                        .map(|l| l.trim().to_string())
                        .filter(|l| !l.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    /// Atomically updates the key pool for an alias and selects a target for the current request.
    /// This method ensures that the rotator's key pool is always in sync with the current configuration.
    async fn update_global_key_pool(
        main_store_arc: Arc<std::sync::RwLock<MainStore>>,
        proxy_alias: &str,
        proxy_group: Option<&str>,
    ) -> ProxyResult<(BackendModelTarget, AiModel)> {
        let backend_targets =
            Self::get_backend_targets(main_store_arc.clone(), proxy_alias, proxy_group)?;
        let group_name = proxy_group.unwrap_or("default");
        let composite_key = format!("{}/{}", group_name, proxy_alias);

        // Atomically update the entire key pool for this alias
        let mut new_key_pool = Vec::new();
        for target in &backend_targets {
            // Get AI model details for this target.
            if let Ok(ai_model) = Self::get_ai_model_details(main_store_arc.clone(), target.id) {
                // Skip providers with no keys (like Ollama)
                if ai_model.api_key.trim().is_empty() {
                    continue;
                }

                let keys: Vec<String> = if ai_model.api_key.contains('\n') {
                    ai_model
                        .api_key
                        .split('\n')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                } else {
                    vec![ai_model.api_key.trim().to_string()]
                };

                for key in keys {
                    // [Gemini] Correctly create the GlobalApiKey struct with all required fields
                    new_key_pool.push(GlobalApiKey {
                        key,
                        provider_id: ai_model.id.unwrap_or_default(),
                        base_url: ai_model.base_url.clone(),
                        model_name: target.model.clone(),
                    });
                }
            }
        }

        log::info!(
            "Atomically replacing key pool for composite key '{}': {} keys from {} providers",
            composite_key,
            new_key_pool.len(),
            backend_targets.len()
        );

        // Call the new atomic replacement method
        CC_PROXY_ROTATOR
            .replace_pool_for_composite_key(&composite_key, new_key_pool)
            .await;

        // The function still needs to select a target for the CURRENT request.
        // We preserve the round-robin logic for selecting the provider for this specific call.
        if backend_targets.is_empty() {
            log::warn!("No backend targets available for alias '{}'.", proxy_alias);
            return Err(CCProxyError::NoBackendTargets(proxy_alias.to_string()));
        }
        let index = CC_PROXY_ROTATOR.get_next_target_index(&composite_key, backend_targets.len());
        let target = &backend_targets[index];

        let ai_model = Self::get_ai_model_details(main_store_arc.clone(), target.id)?;

        Ok((target.clone(), ai_model))
    }

    /// Get the list of backend targets corresponding to the model alias
    fn get_backend_targets(
        main_store_arc: Arc<std::sync::RwLock<MainStore>>,
        proxy_alias: &str,
        proxy_group: Option<&str>,
    ) -> ProxyResult<Vec<BackendModelTarget>> {
        let store_guard = main_store_arc.read().map_err(|e| {
            CCProxyError::StoreLockError(
                t!("db.failed_to_lock_main_store", error = e.to_string()).to_string(),
            )
        })?;

        let proxy_config: ChatCompletionProxyConfig =
            store_guard.get_config(CFG_CHAT_COMPLETION_PROXY, HashMap::new());
        let group = proxy_group.unwrap_or("default");

        proxy_config
            .get(group)
            .and_then(|group_config| {
                // Find the first key that matches the proxy_alias using wildmatch
                group_config
                    .iter()
                    .find(|(key, _)| wildmatch::WildMatch::new(key).matches(proxy_alias))
                    .map(|(_, value)| value)
            })
            .cloned()
            .ok_or_else(|| {
                log::debug!(
                    "proxy configs: {}",
                    serde_json::to_string_pretty(&proxy_config).unwrap_or_default()
                );
                log::warn!(
                    "Proxy alias '{}' not found in group '{}' with wildcard matching.",
                    proxy_alias,
                    group
                );
                CCProxyError::ModelAliasNotFound(proxy_alias.to_string())
            })
    }

    /// Get AI model details by provider_id
    fn get_ai_model_details(
        main_store_arc: Arc<std::sync::RwLock<MainStore>>,
        provider_id: i64,
    ) -> ProxyResult<AiModel> {
        let store_guard = main_store_arc.read().map_err(|e| {
            CCProxyError::StoreLockError(
                t!("db.failed_to_lock_main_store", error = e.to_string()).to_string(),
            )
        })?;

        store_guard
            .config
            .get_ai_model_by_id(provider_id)
            .map_err(|e| {
                CCProxyError::ModelDetailsFetchError(format!("Failed to get model details: {}", e))
            })
    }

    pub async fn get_ai_model_by_provider_and_model(
        main_store_arc: Arc<std::sync::RwLock<MainStore>>,
        provider_id: i64,
        model_id: String,
    ) -> ProxyResult<ProxyModel> {
        let ai_model_detail = Self::get_ai_model_details(main_store_arc.clone(), provider_id)?;

        let api_keys: Vec<String> = ai_model_detail
            .api_key
            .split('\n')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let selected_api_key = if api_keys.is_empty() {
            // If there are no keys after trimming and filtering, return an empty string.
            // This might be the case for Ollama or if keys are not configured.
            "".to_string()
        } else if api_keys.len() == 1 {
            // If there's only one key, no need for rotation.
            api_keys[0].clone()
        } else {
            // If there are multiple keys, use the rotator.
            let composite_key = format!("internal_provider_key_rotation/{}", provider_id);
            let index = CC_PROXY_ROTATOR.get_next_target_index(&composite_key, api_keys.len());
            api_keys
                .get(index)
                .cloned()
                .unwrap_or_else(|| "".to_string()) // Fallback, though index should be valid.
        };

        let custom_params = ai_model_detail
            .models
            .iter()
            .find(|m| m.id == model_id)
            .and_then(|m| m.custom_params.clone());

        Ok(ProxyModel {
            provider: ai_model_detail.name.clone(),
            chat_protocol: ai_model_detail.api_protocol.try_into().unwrap_or_default(),
            base_url: ai_model_detail.base_url,
            model: model_id,
            api_key: selected_api_key,
            model_metadata: ai_model_detail.metadata.clone(),
            custom_params,
            prompt_injection: "off".to_string(),
            prompt_injection_position: Some("system".to_string()),
            prompt_text: "".to_string(),
            tool_filter: HashMap::new(),
            prompt_replace: Vec::new(),
            temperature: 1.0,
            max_tokens: ai_model_detail.max_tokens,
            presence_penalty: ai_model_detail
                .metadata
                .as_ref()
                .and_then(|m| m.get("presencePenalty"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0) as f32,
            frequency_penalty: ai_model_detail
                .metadata
                .as_ref()
                .and_then(|m| m.get("frequencyPenalty"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0) as f32,
            top_p: ai_model_detail.top_p,
            top_k: ai_model_detail.top_k,
            stop: ai_model_detail
                .metadata
                .as_ref()
                .and_then(|m| m.get("stop"))
                .and_then(|v| v.as_str())
                .map(|s| {
                    s.lines()
                        .map(|l| l.trim().to_string())
                        .filter(|l| !l.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    fn get_proxy_group(
        main_store_arc: Arc<std::sync::RwLock<MainStore>>,
        group_name: &str,
    ) -> ProxyResult<ProxyGroup> {
        let store_guard = main_store_arc.read().map_err(|e| {
            CCProxyError::StoreLockError(
                t!("db.failed_to_lock_main_store", error = e.to_string()).to_string(),
            )
        })?;
        store_guard
            .config
            .get_proxy_group_by_name(group_name)
            .map_err(|e| {
                CCProxyError::ModelDetailsFetchError(format!("Failed to get proxy group: {}", e))
            })
    }

    pub fn build_http_client(
        main_store_arc: Arc<std::sync::RwLock<MainStore>>,
        mut metadata: Option<serde_json::Value>,
    ) -> ProxyResult<Client> {
        let mut client_builder = Client::builder();
        let _ = setup_chat_proxy(main_store_arc.clone(), &mut metadata);
        let proxy_type = get_proxy_type(metadata);

        match proxy_type {
            ProxyType::None => {
                client_builder = client_builder.no_proxy();
            }
            ProxyType::Http(proxy_url, proxy_username, proxy_password) => {
                log::debug!("Using proxy: {}", proxy_url);
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
    if name_str == "x-provider-id" || name_str == "x-model-id" || name_str == "x-internal-request" {
        return false;
    }

    return name_str == "x-request-id"
            || name_str == "user-agent"
            || name_str == "accept-language"
            || name_str == "http-referer"
            || name_str == "conversation-id"
            || name_str == "session-id"
            || name_str == "traceparent"
            || name_str.starts_with("cs-")
            // Add other custom headers you want to forward, e.g., "x-custom-tenant-id"
            || (name_str.starts_with("x-") && !name_str.starts_with("x-api-"));
}

pub fn get_provider_chat_full_url(
    protocol: ChatProtocol,
    base_url: &str,
    model_id: &str,
    api_key: &str,
    is_streaming_request: bool,
) -> String {
    match protocol {
        ChatProtocol::OpenAI => {
            format!("{}/chat/completions", base_url.trim_end_matches('/'))
        }
        ChatProtocol::Ollama => {
            format!("{}/api/chat", base_url.trim_end_matches('/'))
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
            if is_streaming_request {
                format!(
                    "{}/models/{}:streamGenerateContent?alt=sse&key={}",
                    base_url.trim_end_matches('/'),
                    model_id,
                    api_key
                )
            } else {
                format!(
                    "{}/models/{}:generateContent?key={}",
                    base_url.trim_end_matches('/'),
                    model_id,
                    api_key
                )
            }
        }
    }
}

pub fn get_tool_id() -> String {
    format!("tool_{}", &uuid::Uuid::new_v4().to_string()[..8])
}

pub fn get_msg_id() -> String {
    format!("msg_{}", &uuid::Uuid::new_v4().to_string()[..8])
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    #[test]
    fn test_wildmatch_logic() {
        let mut group_config = IndexMap::new();
        // Insert rules from most specific to most general
        group_config.insert("claude-3-opus-20240229".to_string(), "exact_opus");
        group_config.insert("claude-3-sonnet*".to_string(), "suffix_sonnet");
        group_config.insert("*-haiku-20240307".to_string(), "prefix_haiku");
        group_config.insert("gemini-*-pro".to_string(), "infix_gemini");
        group_config.insert("qwen-v1.?-chat".to_string(), "single_char_qwen");
        group_config.insert("*".to_string(), "wildcard_catch_all");

        // Test exact match (should be found first)
        let proxy_alias = "claude-3-opus-20240229";
        let result = group_config
            .iter()
            .find(|(key, _)| wildmatch::WildMatch::new(key).matches(proxy_alias))
            .map(|(_, value)| *value);
        assert_eq!(result, Some("exact_opus"));

        // Test suffix match
        let proxy_alias = "claude-3-sonnet-20240229";
        let result = group_config
            .iter()
            .find(|(key, _)| wildmatch::WildMatch::new(key).matches(proxy_alias))
            .map(|(_, value)| *value);
        assert_eq!(result, Some("suffix_sonnet"));

        // Test prefix match
        let proxy_alias = "claude-3-haiku-20240307";
        let result = group_config
            .iter()
            .find(|(key, _)| wildmatch::WildMatch::new(key).matches(proxy_alias))
            .map(|(_, value)| *value);
        assert_eq!(result, Some("prefix_haiku"));

        // Test infix match
        let proxy_alias = "gemini-1.5-pro";
        let result = group_config
            .iter()
            .find(|(key, _)| wildmatch::WildMatch::new(key).matches(proxy_alias))
            .map(|(_, value)| *value);
        assert_eq!(result, Some("infix_gemini"));

        // Test single character (?) match
        let proxy_alias = "qwen-v1.5-chat";
        let result = group_config
            .iter()
            .find(|(key, _)| wildmatch::WildMatch::new(key).matches(proxy_alias))
            .map(|(_, value)| *value);
        assert_eq!(result, Some("single_char_qwen"));

        // Test catch-all (*)
        let proxy_alias = "some-other-model-that-does-not-match-others";
        let result = group_config
            .iter()
            .find(|(key, _)| wildmatch::WildMatch::new(key).matches(proxy_alias))
            .map(|(_, value)| *value);
        assert_eq!(result, Some("wildcard_catch_all"));

        // Test no match (when no catch-all is present)
        let mut group_config_no_catch_all = IndexMap::new();
        group_config_no_catch_all.insert("claude-3-opus-20240229".to_string(), "exact_opus");
        let proxy_alias = "gpt-4-turbo";
        let result = group_config_no_catch_all
            .iter()
            .find(|(key, _)| wildmatch::WildMatch::new(key).matches(proxy_alias))
            .map(|(_, value)| *value);
        assert_eq!(result, None);
    }
}
