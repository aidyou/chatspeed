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

/// Macro to unify sampling parameter merging logic across different data structures (JSON Map vs Struct).
///
/// This macro implements the following hierarchy:
/// 1. **Client Priority**: If the client provides a parameter, it is kept (and potentially scaled).
/// 2. **Proxy Fallback**: If the client omits a parameter, the proxy injects its configured value
///    only if that value is considered "valid" (usually non-zero or non-default).
///
/// # Arguments
/// * `$target`: The object to mutate (either a `serde_json::Map` or a `UnifiedRequest` struct).
/// * `$proxy`: The `ProxyModel` containing the configuration.
/// * `$mode`: Either `json` or `struct`, determining how fields are accessed and modified.
#[macro_export]
macro_rules! apply_sampling_params {
    ($target:expr, $proxy:expr, json) => {
        // --- Temperature Logic ---
        let client_temp = $target.get("temperature").and_then(|v| v.as_f64());
        if let Some(t) = client_temp {
            if $proxy.temp_ratio != 1.0 {
                $target.insert(
                    "temperature".into(),
                    serde_json::json!(t * $proxy.temp_ratio as f64),
                );
            }
        } else if let Some(model_temp) = $proxy.temperature {
            // Valid range for temperature is typically 0.0 to 2.0
            if model_temp >= 0.0 && model_temp <= 2.0 {
                $target.insert("temperature".into(), serde_json::json!(model_temp));
            }
        }

        // --- Max Tokens ---
        if $target.get("max_tokens").is_none() && $target.get("max_completion_tokens").is_none() {
            if let Some(v) = $proxy.max_tokens {
                if v > 0 {
                    $target.insert("max_tokens".into(), serde_json::json!(v));
                }
            }
        }

        // --- Top P (Valid range: (0, 1]) ---
        if $target.get("top_p").is_none() {
            if let Some(v) = $proxy.top_p {
                if v > 0.0 && v <= 1.0 {
                    $target.insert("top_p".into(), serde_json::json!(v));
                }
            }
        }

        // --- Top K (Valid if > 0) ---
        if $target.get("top_k").is_none() {
            if let Some(v) = $proxy.top_k {
                if v > 0 {
                    $target.insert("top_k".into(), serde_json::json!(v));
                }
            }
        }

        // --- Penalties (Valid range: [-2, 2]) ---
        // Zero -> unset
        if $target.get("presence_penalty").is_none() {
            if let Some(v) = $proxy.presence_penalty {
                if v >= -2.0 && v <= 2.0 && v != 0.0 {
                    $target.insert("presence_penalty".into(), serde_json::json!(v));
                }
            }
        }
        if $target.get("frequency_penalty").is_none() {
            if let Some(v) = $proxy.frequency_penalty {
                if v >= -2.0 && v <= 2.0 && v != 0.0 {
                    $target.insert("frequency_penalty".into(), serde_json::json!(v));
                }
            }
        }

        // --- Stop Sequences ---
        if $target.get("stop").is_none() && !$proxy.stop.is_empty() {
            $target.insert("stop".into(), serde_json::json!($proxy.stop));
        }
    };

    ($target:expr, $proxy:expr, struct) => {
        // --- Temperature Logic ---
        if let Some(t) = $target.temperature {
            if $proxy.temp_ratio != 1.0 {
                $target.temperature = Some(t * $proxy.temp_ratio);
            }
        } else if let Some(model_temp) = $proxy.temperature {
            if model_temp >= 0.0 && model_temp <= 2.0 {
                $target.temperature = Some(model_temp);
            }
        }

        // --- Max Tokens ---
        if $target.max_tokens.is_none() {
            if let Some(v) = $proxy.max_tokens {
                if v > 0 {
                    $target.max_tokens = Some(v);
                }
            }
        }

        // --- Top P ---
        if $target.top_p.is_none() {
            if let Some(v) = $proxy.top_p {
                if v > 0.0 && v <= 1.0 {
                    $target.top_p = Some(v);
                }
            }
        }

        // --- Top K ---
        if $target.top_k.is_none() {
            if let Some(v) = $proxy.top_k {
                if v > 0 {
                    $target.top_k = Some(v);
                }
            }
        }

        // --- Penalties ---
        // Zero -> unset
        if $target.presence_penalty.is_none() {
            if let Some(v) = $proxy.presence_penalty {
                if v >= -2.0 && v <= 2.0 && v != 0.0 {
                    $target.presence_penalty = Some(v);
                }
            }
        }
        if $target.frequency_penalty.is_none() {
            if let Some(v) = $proxy.frequency_penalty {
                if v >= -2.0 && v <= 2.0 && v != 0.0 {
                    $target.frequency_penalty = Some(v);
                }
            }
        }

        // --- Stop Sequences ---
        if $target.stop_sequences.is_none() && !$proxy.stop.is_empty() {
            $target.stop_sequences = Some($proxy.stop.clone());
        }
    };
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
        // We now get the actual matched alias key from the configuration
        let (matched_alias_key, backend_target, ai_model_detail) =
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
        let temp_ratio = group_config
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

        // Read tool_compat_mode from metadata
        let tool_compat_mode_override = group_config
            .as_ref()
            .map(|g| {
                g.metadata
                    .as_ref()
                    .and_then(|m| m.get("toolCompatMode"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or(None);

        // Ollama hasn't api key
        if ai_model_detail.api_protocol == ChatProtocol::Ollama.to_string() {
            let custom_params = ai_model_detail
                .models
                .iter()
                .find(|m| m.id == backend_target.model)
                .and_then(|m| m.custom_params.clone());

            let metadata = ai_model_detail.metadata.as_ref();

            return Ok(ProxyModel {
                client_alias: matched_alias_key,
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
                temp_ratio,
                max_tokens: if ai_model_detail.max_tokens > 0 {
                    Some(ai_model_detail.max_tokens)
                } else {
                    None
                },
                temperature: metadata
                    .and_then(|m| m.get("temperature"))
                    .and_then(|v| v.as_f64())
                    .map(|v| v as f32),
                presence_penalty: metadata
                    .and_then(|m| m.get("presencePenalty"))
                    .and_then(|v| v.as_f64())
                    .map(|v| v as f32),
                frequency_penalty: metadata
                    .and_then(|m| m.get("frequencyPenalty"))
                    .and_then(|v| v.as_f64())
                    .map(|v| v as f32),
                top_p: if ai_model_detail.top_p > 0.0 && ai_model_detail.top_p < 1.0 {
                    Some(ai_model_detail.top_p)
                } else {
                    None
                },
                top_k: if ai_model_detail.top_k > 0 {
                    Some(ai_model_detail.top_k)
                } else {
                    None
                },
                prompt_replace,
                stop: metadata
                    .and_then(|m| m.get("stop"))
                    .and_then(|v| v.as_str())
                    .map(|s| {
                        s.lines()
                            .map(|l| l.trim().to_string())
                            .filter(|l| !l.is_empty())
                            .collect()
                    })
                    .unwrap_or_default(),
                tool_compat_mode: tool_compat_mode_override.map(|s| s.to_string()),
            });
        }

        // Use matched_alias_key for rotation logic to ensure counter persistence across dynamic model IDs
        let composite_key = format!("{}/{}", group_name, matched_alias_key);

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
            &matched_alias_key,
            &ai_model_details.name,
            &ai_model_details.base_url,
            &ai_model_details.api_protocol,
            &global_key.key[std::cmp::max(0, global_key.key.len() - 8)..] // Log last 8 chars for debugging
        );

        if ai_model_details.base_url.is_empty() {
            log::error!(
                "Target AI provider base_url is empty for alias '{}'",
                matched_alias_key
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

        let metadata = ai_model_details.metadata.as_ref();

        Ok(ProxyModel {
            client_alias: matched_alias_key,
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
            temp_ratio,
            max_tokens: if ai_model_details.max_tokens > 0 {
                Some(ai_model_details.max_tokens)
            } else {
                None
            },
            temperature: metadata
                .and_then(|m| m.get("temperature"))
                .and_then(|v| v.as_f64())
                .map(|v| v as f32),
            presence_penalty: metadata
                .and_then(|m| m.get("presencePenalty"))
                .and_then(|v| v.as_f64())
                .map(|v| v as f32),
            frequency_penalty: metadata
                .and_then(|m| m.get("frequencyPenalty"))
                .and_then(|v| v.as_f64())
                .map(|v| v as f32),
            top_p: if ai_model_details.top_p > 0.0 && ai_model_details.top_p < 1.0 {
                Some(ai_model_details.top_p)
            } else {
                None
            },
            top_k: if ai_model_details.top_k > 0 {
                Some(ai_model_details.top_k)
            } else {
                None
            },
            prompt_replace,
            stop: metadata
                .and_then(|m| m.get("stop"))
                .and_then(|v| v.as_str())
                .map(|s| {
                    s.lines()
                        .map(|l| l.trim().to_string())
                        .filter(|l| !l.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            tool_compat_mode: tool_compat_mode_override.map(|s| s.to_string()),
        })
    }

    /// Atomically updates the key pool for an alias and selects a target for the current request.
    /// This method ensures that the rotator's key pool is always in sync with the current configuration.
    async fn update_global_key_pool(
        main_store_arc: Arc<std::sync::RwLock<MainStore>>,
        proxy_alias: &str,
        proxy_group: Option<&str>,
    ) -> ProxyResult<(String, BackendModelTarget, AiModel)> {
        let (matched_alias_key, backend_targets) =
            Self::get_backend_targets(main_store_arc.clone(), proxy_alias, proxy_group)?;
        let group_name = proxy_group.unwrap_or("default");
        let composite_key = format!("{}/{}", group_name, matched_alias_key);

        // Group targets by provider to avoid weight multiplication.
        // Use BTreeMap for stable iteration order.
        let mut provider_to_models: std::collections::BTreeMap<i64, Vec<String>> =
            std::collections::BTreeMap::new();
        for target in &backend_targets {
            provider_to_models
                .entry(target.id)
                .or_default()
                .push(target.model.clone());
        }

        // Atomically update the entire key pool for this alias
        //
        // [Logic Refactor Explanation - 2026-01-30]
        //
        // Previous Issue (Weight Explosion):
        // The old logic iterated: `for target in targets { for key in provider_keys { push(key, target) } }`.
        // If a provider (e.g., ModelScope) was configured with 3 targets (e.g., via wildcards or multiple alias entries)
        // and had 3 keys, it would create 3 * 3 = 9 entries in the pool.
        // If another provider (e.g., iFlow) had 1 target and 1 key, it had 1 entry.
        // This resulted in a 9:1 traffic skew, whereas the user expected a 3:1 skew based on key count (3 keys vs 1 key).
        //
        // Current Logic (Key-Centric Rotation):
        // 1. We group targets by Provider ID first.
        // 2. We ensure each PHYSICAL KEY from a provider is added to the global pool exactly ONCE.
        //    This ensures that 3 keys = 3 entries, regardless of how many model aliases map to this provider.
        // 3. To support multiple models for a single provider (e.g., "ds-3.2" and "ds-3.2-chat"),
        //    we use a secondary round-robin counter (`model_rot`) to rotate the model name used by that key.
        //
        // Result: Traffic distribution is strictly proportional to the number of valid API keys.
        let mut new_key_pool = Vec::new();
        for (provider_id, models) in provider_to_models {
            if let Ok(ai_model) = Self::get_ai_model_details(main_store_arc.clone(), provider_id) {
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

                // Use a dedicated counter for rotating models within this provider to ensure
                // that even if keys < models, all models are eventually used without inflating the key's weight.
                let model_rot_key = format!("{}:{}:model_rot", composite_key, provider_id);
                let model_rot = CC_PROXY_ROTATOR.get_next_target_index(&model_rot_key, models.len());

                // Each unique key is added exactly ONCE to the global pool.
                for (key_idx, key) in keys.into_iter().enumerate() {
                    let model_idx = (key_idx + model_rot) % models.len();
                    new_key_pool.push(GlobalApiKey {
                        key,
                        provider_id: ai_model.id.unwrap_or_default(),
                        base_url: ai_model.base_url.clone(),
                        model_name: models[model_idx].clone(),
                    });
                }
            }
        }

        // Call the atomic replacement method
        CC_PROXY_ROTATOR
            .replace_pool_for_composite_key(&composite_key, new_key_pool)
            .await;

        // The function still needs to select a target for the CURRENT request (mostly for early returns/Ollama).
        if backend_targets.is_empty() {
            log::warn!("No backend targets available for alias '{}'.", proxy_alias);
            return Err(CCProxyError::NoBackendTargets(proxy_alias.to_string()));
        }
        let index = CC_PROXY_ROTATOR.get_next_target_index(&composite_key, backend_targets.len());
        let target = &backend_targets[index];

        let ai_model = Self::get_ai_model_details(main_store_arc.clone(), target.id)?;

        Ok((matched_alias_key, target.clone(), ai_model))
    }

    /// Get the list of backend targets corresponding to the model alias
    fn get_backend_targets(
        main_store_arc: Arc<std::sync::RwLock<MainStore>>,
        proxy_alias: &str,
        proxy_group: Option<&str>,
    ) -> ProxyResult<(String, Vec<BackendModelTarget>)> {
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
                    .map(|(key, value)| (key.clone(), value.clone()))
            })
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

        let metadata = ai_model_detail.metadata.as_ref();

        Ok(ProxyModel {
            client_alias: model_id.clone(), // For internal requests, alias is the ID
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
            temp_ratio: 1.0,
            max_tokens: if ai_model_detail.max_tokens > 0 {
                Some(ai_model_detail.max_tokens)
            } else {
                None
            },
            temperature: metadata
                .and_then(|m| m.get("temperature"))
                .and_then(|v| v.as_f64())
                .map(|v| v as f32),
            presence_penalty: metadata
                .and_then(|m| m.get("presencePenalty"))
                .and_then(|v| v.as_f64())
                .map(|v| v as f32),
            frequency_penalty: metadata
                .and_then(|m| m.get("frequencyPenalty"))
                .and_then(|v| v.as_f64())
                .map(|v| v as f32),
            top_p: if ai_model_detail.top_p > 0.0 && ai_model_detail.top_p < 1.0 {
                Some(ai_model_detail.top_p)
            } else {
                None
            },
            top_k: if ai_model_detail.top_k > 0 {
                Some(ai_model_detail.top_k)
            } else {
                None
            },
            stop: metadata
                .and_then(|m| m.get("stop"))
                .and_then(|v| v.as_str())
                .map(|s| {
                    s.lines()
                        .map(|l| l.trim().to_string())
                        .filter(|l| !l.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            tool_compat_mode: None,
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

    /// Unified parameter merging logic for JSON bodies (Direct Forward mode).
    /// Precedence: Client Body > Proxy Configuration (Valid Non-Defaults)
    pub fn merge_parameters_json(body: &mut serde_json::Value, proxy_model: &ProxyModel) {
        if let Some(body_map) = body.as_object_mut() {
            apply_sampling_params!(body_map, proxy_model, json);
        }
    }

    /// Unified parameter merging logic for UnifiedRequest (Protocol Conversion mode).
    /// Precedence: Client Request > Proxy Configuration (Valid Non-Defaults)
    pub fn merge_parameters_unified(
        req: &mut crate::ccproxy::adapter::unified::UnifiedRequest,
        proxy_model: &ProxyModel,
    ) {
        apply_sampling_params!(req, proxy_model, struct);
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

    /// Unified header injection logic for all proxy requests.
    /// Order of precedence: Metadata < Protocol Defaults < Client Overrides (cs-*)
    pub fn inject_proxy_headers(
        final_headers: &mut reqwest::header::HeaderMap,
        client_headers: &http::HeaderMap,
        proxy_model: &ProxyModel,
        message_id: &str,
    ) {
        // 1. Forward relevant headers from client (Override everything)
        for (name, value) in client_headers.iter() {
            let name_str = name.as_str().to_lowercase();
            if should_forward_header(&name_str) {
                // The "cs-" prefix is added to custom headers defined by users in the backend;
                // it MUST be stripped before forwarding to the AI server to restore the original name.
                let final_name = if name_str.starts_with("cs-") {
                    &name_str[3..]
                } else {
                    &name_str
                };

                if let (Ok(reqwest_name), Ok(reqwest_value)) = (
                    reqwest::header::HeaderName::from_bytes(final_name.as_bytes()),
                    reqwest::header::HeaderValue::from_bytes(value.as_ref()),
                ) {
                    final_headers.insert(reqwest_name, reqwest_value);
                }
            }
        }

        // 2. Add custom headers from model metadata (Default values)
        let custom_headers =
            crate::ai::util::process_custom_headers(&proxy_model.model_metadata, message_id);
        for (k, v) in custom_headers {
            if !should_forward_header(&k) {
                continue;
            }
            let final_key = if k.to_lowercase().starts_with("cs-") {
                &k[3..]
            } else {
                &k
            };
            if let (Ok(n), Ok(h)) = (
                reqwest::header::HeaderName::from_bytes(final_key.as_bytes()),
                reqwest::header::HeaderValue::from_bytes(v.as_bytes()),
            ) {
                final_headers.insert(n, h);
            }
        }

        // 3. Add protocol-specific authentication and version headers
        match proxy_model.chat_protocol {
            ChatProtocol::OpenAI | ChatProtocol::HuggingFace => {
                if !proxy_model.api_key.is_empty() {
                    if let Ok(h) = reqwest::header::HeaderValue::from_str(&format!(
                        "Bearer {}",
                        proxy_model.api_key
                    )) {
                        final_headers.insert(reqwest::header::AUTHORIZATION, h);
                    }
                }
            }
            ChatProtocol::Claude => {
                if let Ok(h) = reqwest::header::HeaderValue::from_str(&proxy_model.api_key) {
                    final_headers.insert(reqwest::header::HeaderName::from_static("x-api-key"), h);
                }
                final_headers.insert(
                    reqwest::header::HeaderName::from_static("anthropic-version"),
                    reqwest::header::HeaderValue::from_static("2023-06-01"),
                );
            }
            ChatProtocol::Gemini => {} // API key is in URL
            ChatProtocol::Ollama => {} // No auth
        }
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
    // 1. Block internal routing headers
    if name_str == "x-cs-provider-id"
        || name_str == "x-cs-model-id"
        || name_str == "x-cs-internal-request"
    {
        return false;
    }

    // 2. Block restricted hop-by-hop or transport-managed headers
    // Even if they have 'cs-' prefix, we should strip it and check the base name
    let base_name = if name_str.starts_with("cs-") {
        &name_str[3..]
    } else {
        name_str
    };

    let restricted_headers = [
        "host",
        "connection",
        "content-length",
        "transfer-encoding",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "upgrade",
        "expect",
    ];

    if restricted_headers.contains(&base_name) {
        return false;
    }

    // 3. Allowlist for specific headers and pattern-based forwarding
    return name_str == "x-request-id"
        || name_str == "user-agent"
        || name_str == "accept-language"
        || name_str == "http-referer"
        || name_str == "conversation-id"
        || name_str == "session-id"
        || name_str == "traceparent"
        || name_str.starts_with("cs-")
        || (name_str.starts_with("x-") && !name_str.starts_with("x-api-"));
}

pub fn get_provider_chat_full_url(
    protocol: ChatProtocol,
    base_url: &str,
    model_id: &str,
    api_key: &str,
    is_streaming_request: bool,
) -> String {
    let clean_model_id = model_id.trim();
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
                    base, clean_model_id
                )
            })
            .unwrap_or_else(|| {
                format!(
                    "https://router.huggingface.co/hf-inference/models/{}/v1/chat/completions",
                    clean_model_id
                )
            }),
        ChatProtocol::Claude => format!("{}/messages", base_url.trim_end_matches('/')),
        ChatProtocol::Gemini => {
            if is_streaming_request {
                format!(
                    "{}/models/{}:streamGenerateContent?alt=sse&key={}",
                    base_url.trim_end_matches('/'),
                    clean_model_id,
                    api_key
                )
            } else {
                format!(
                    "{}/models/{}:generateContent?key={}",
                    base_url.trim_end_matches('/'),
                    clean_model_id,
                    api_key
                )
            }
        }
    }
}

pub fn get_provider_embedding_full_url(
    protocol: ChatProtocol,
    base_url: &str,
    model_id: &str,
    api_key: &str,
) -> String {
    let clean_model_id = model_id.trim();
    match protocol {
        ChatProtocol::OpenAI => {
            format!("{}/embeddings", base_url.trim_end_matches('/'))
        }
        ChatProtocol::Ollama => {
            format!("{}/api/embed", base_url.trim_end_matches('/'))
        }
        ChatProtocol::HuggingFace => format!(
            "https://api-inference.huggingface.co/pipeline/feature-extraction/{}",
            clean_model_id
        ),
        ChatProtocol::Gemini => {
            // Robust base URL extraction: remove any suffix starting with ':'
            // But be careful not to split the 'https://' protocol part.
            let base = if let Some(idx) = base_url.rfind(':') {
                if idx > 6 {
                    // Likely an action suffix like :generateContent
                    &base_url[..idx]
                } else {
                    // Likely the colon in https://
                    base_url
                }
            } else {
                base_url
            };
            format!(
                "{}/models/{}:embedContent?key={}",
                base.trim_end_matches('/'),
                clean_model_id,
                api_key
            )
        }
        ChatProtocol::Claude => String::new(),
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
