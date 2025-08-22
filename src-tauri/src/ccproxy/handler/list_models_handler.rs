use axum::Json;
use lazy_static::*;
use regex::Regex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc};

use crate::{
    ccproxy::{
        errors::{CCProxyError, ProxyResult},
        types::BackendModelTarget,
    },
    db::MainStore,
};

// =================================================
//  chat completion proxy
// =================================================
/// Handles the `/v1/models` (list models) request for openai and claude.
/// Reads `chat_completion_proxy` from `MainStore`.
pub async fn handle_list_models(
    group_name: Option<String>,
    main_store: Arc<std::sync::RwLock<MainStore>>,
) -> ProxyResult<Json<Value>> {
    let group = group_name.as_deref().unwrap_or("default");
    let config: HashMap<String, Vec<BackendModelTarget>> = {
        let cfg: HashMap<String, HashMap<String, Vec<BackendModelTarget>>> = main_store
            .read()
            .map_err(|e| CCProxyError::StoreLockError(e.to_string()))?
            .get_config("chat_completion_proxy", HashMap::new());

        cfg.get(group)
            .cloned()
            .ok_or_else(|| {
                log::warn!("Proxy group '{}' not found.", group);
                CCProxyError::ModelAliasNotFound(group.to_string())
            })
            .unwrap_or(HashMap::new())
    };

    let keys = config.keys().cloned().collect::<Vec<String>>();
    let mut models_info: HashMap<String, (i32, String)> = HashMap::new();
    let store = main_store
        .read()
        .map_err(|e| CCProxyError::StoreLockError(e.to_string()))?;

    for (alias, target) in config.into_iter() {
        // If the alias has already been processed and added to `models_info`, skip it.
        // This prevents re-processing the same alias.
        if models_info.contains_key(&alias) {
            continue;
        }

        // Iterate through the `target` items associated with the current `alias`.
        for t in target.into_iter() {
            // Only consider target items with a positive ID.
            if t.id > 0 {
                // Attempt to retrieve the AI model from the store using the target item's ID.
                if let Ok(model) = store.config.get_ai_model_by_id(t.id) {
                    // If a model is successfully retrieved, insert its details into `models_info`.
                    // We clone `alias` because the `for` loop consumes it by value,
                    // and we need to move it into the HashMap.
                    models_info.insert(alias.clone(), (model.max_tokens, model.name.clone()));
                    // Since we found a model for this alias, we can stop searching
                    // further in `target` for this specific alias.
                    break;
                }
            }
        }
    }

    let mut sorted_keys = keys.clone();
    sorted_keys.sort();

    if models_info.is_empty() {
        log::info!("'chat_completion_proxy' configuration is empty or not found. Returning empty model list.");
    };

    let models_list: Vec<Value> = sorted_keys
        .into_iter()
        .map(|alias| {
            let (max_token, provider) = if let Some(m) = models_info.get(&alias) {
                m.clone()
            } else {
                (-1, "ccproxy".to_string())
            };
            let now = std::time::SystemTime::now();
            let created_unix = now
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            json!({
                "id": alias.clone(),
                "object": "model".to_string(),
                "type": "model".to_string(),
                "created": created_unix,
                "created_at": chrono::Utc::now().to_rfc3339(),
                "display_name": alias.clone(),
                "owned_by": provider,
                "max_tokens": max_token,
                "powered_by": "chatspeed ccproxy".to_string(),
            })
        })
        .collect();

    let first_id = models_list
        .first()
        .and_then(|m| m["id"].as_str())
        .map(|s| s.to_string());
    let last_id = models_list
        .last()
        .and_then(|m| m["id"].as_str())
        .map(|s| s.to_string());

    let response = json!( {
        "object": "list".to_string(),
        "data": models_list,
        "first_id": first_id,
        "has_more": false,
        "last_id": last_id,
    });

    Ok(Json(response))
}

/// List proxied models handler for ollama
/// GET `/api/tags`
pub async fn handle_ollama_tags(
    group_name: Option<String>,
    main_store: Arc<std::sync::RwLock<MainStore>>,
) -> ProxyResult<Json<Value>> {
    let group = group_name.as_deref().unwrap_or("default");
    let config: HashMap<String, Vec<BackendModelTarget>> = {
        let cfg: HashMap<String, HashMap<String, Vec<BackendModelTarget>>> = main_store
            .read()
            .map_err(|e| CCProxyError::StoreLockError(e.to_string()))?
            .get_config("chat_completion_proxy", HashMap::new());

        cfg.get(group)
            .cloned()
            .ok_or_else(|| {
                log::warn!("Proxy group '{}' not found.", group);
                CCProxyError::ModelAliasNotFound(group.to_string())
            })
            .unwrap_or(HashMap::new())
    };

    let keys = config.keys().cloned().collect::<Vec<String>>();
    let mut models: HashMap<String, (i32, String)> = HashMap::new();
    let store = main_store
        .read()
        .map_err(|e| CCProxyError::StoreLockError(e.to_string()))?;

    for (alias, target) in config.into_iter() {
        // If the alias has already been processed and added to `models`, skip it.
        // This prevents re-processing the same alias.
        if models.contains_key(&alias) {
            continue;
        }

        // Iterate through the `target` items associated with the current `alias`.
        for t in target.into_iter() {
            // Only consider target items with a positive ID.
            if t.id > 0 {
                // Attempt to retrieve the AI model from the store using the target item's ID.
                if let Ok(model) = store.config.get_ai_model_by_id(t.id) {
                    // If a model is successfully retrieved, insert its details into `models`.
                    // We clone `alias` because the `for` loop consumes it by value,
                    // and we need to move it into the HashMap.
                    models.insert(alias.clone(), (model.max_tokens, model.name.clone()));
                    // Since we found a model for this alias, we can stop searching
                    // further in `target` for this specific alias.
                    break;
                }
            }
        }
    }

    // Sort keys to ensure consistent order of models
    let mut sorted_keys = keys.clone();
    sorted_keys.sort();
    let models: Vec<Value> = sorted_keys
        .into_iter()
        .map(|alias| {
            let (max_token, provider) = if let Some(m) = models.get(&alias) {
                m.clone()
            } else {
                (-1, "ccproxy".to_string())
            };
            // Generate SHA256 digest based on current timestamp and model info
            let mut hasher = Sha256::new();
            let timestamp = chrono::Utc::now().to_rfc3339();
            hasher.update(format!("{}:{}:{}", alias, provider, timestamp));
            let digest_bytes = hasher.finalize();
            let digest = format!("{:x}", digest_bytes);

            // Derive model family from model name
            let family = derive_model_family(&alias);
            let families = vec![family.clone(), provider.clone()];

            // Determine appropriate quantization level
            let quantization_level = determine_quantization_level(&alias, &provider);

            json!( {
                "name": alias.clone(),
                "model": alias.clone(),
                "modified_at": timestamp,
                "size": 4683075271_u64,
                "digest": digest,
                "details": {
                    "parent_model": "",
                    "format": "gguf",
                    "family": family,
                    "families": families,
                    "parameter_size": "480B",
                    "quantization_level": quantization_level
                },
                "owned_by": "chatspeed ccproxy".to_string(),
                "max_tokens": max_token,
                "powered_by": "chatspeed ccproxy".to_string(),
            })
        })
        .collect();

    if models.is_empty() {
        log::info!("'chat_completion_proxy' configuration is empty or not found. Returning empty model list.");
    };

    let response = json!( {
        "models": models,
    });

    Ok(Json(response))
}

lazy_static! {
    static ref MODEL_REGEX: Regex = Regex::new(r"^[a-zA-Z0-9]+(?:-[a-zA-Z0-9]+)*").unwrap();
    static ref QUANT_REGEX: Regex =
        Regex::new(r"[_-](q[2-8](?:_[a-zA-Z0-9])?|[fF](?:16|32))").unwrap();
}
/// Derive model family from model name using regex for better accuracy.
/// For example: "qwen2-7b-instruct" should belong to "qwen2" family.
fn derive_model_family(model_name: &str) -> String {
    // This regex aims to capture the base name of the model before any specifiers
    // like size (7b), version (v1.5), or type (instruct).
    // It looks for a pattern of (word-word-...) and captures that.
    if let Some(mat) = MODEL_REGEX.find(model_name) {
        // Further refinement to remove common suffixes that are not part of the family name
        let family = mat.as_str();
        return family
            .trim_end_matches("-instruct")
            .trim_end_matches("-chat")
            .trim_end_matches("-base")
            .to_string();
    }

    // Fallback for names that don't fit the pattern, e.g., "gemma"
    model_name.to_string()
}

/// Determine appropriate quantization level based on model name using regex.
fn determine_quantization_level(model_name: &str, provider: &str) -> String {
    // For closed-source providers where we don't know the specifics
    let closed_source_providers = ["openai", "anthropic", "google"];
    if closed_source_providers.contains(&provider) {
        return "N/A".to_string();
    }

    let model_lower = model_name.to_lowercase();

    // Regex for quantization levels (e.g., q4_k_m, q8_0, f16)
    // This looks for common quantization patterns like q2-q8 with optional suffixes, or f16/f32.

    if let Some(mat) = QUANT_REGEX.find(&model_lower) {
        // Extract the matched group and format it to uppercase standard
        let matched_str = mat.as_str().trim_start_matches(|c| c == '-' || c == '_');
        return matched_str.to_uppercase();
    }

    // Default quantization level if no specific pattern is found
    "Unknown".to_string()
}

/// Handles the `/v1beta/models` (list models) request for gemini.
pub async fn handle_gemini_list_models(
    group_name: Option<String>,
    main_store: Arc<std::sync::RwLock<MainStore>>,
) -> ProxyResult<Json<Value>> {
    let group = group_name.as_deref().unwrap_or("default");
    let config: HashMap<String, Vec<BackendModelTarget>> = {
        let cfg: HashMap<String, HashMap<String, Vec<BackendModelTarget>>> = main_store
            .read()
            .map_err(|e| CCProxyError::StoreLockError(e.to_string()))?
            .get_config("chat_completion_proxy", HashMap::new());

        cfg.get(group)
            .cloned()
            .ok_or_else(|| {
                log::warn!("Proxy group '{}' not found.", group);
                CCProxyError::ModelAliasNotFound(group.to_string())
            })
            .unwrap_or(HashMap::new())
    };

    let keys = config.keys().cloned().collect::<Vec<String>>();
    let mut models: HashMap<String, (i32, String)> = HashMap::new();
    let store = main_store
        .read()
        .map_err(|e| CCProxyError::StoreLockError(e.to_string()))?;

    for (alias, target) in config.into_iter() {
        if models.contains_key(&alias) {
            continue;
        }

        for t in target.into_iter() {
            if t.id > 0 {
                if let Ok(model) = store.config.get_ai_model_by_id(t.id) {
                    models.insert(alias.clone(), (model.max_tokens, model.name.clone()));
                    break;
                }
            }
        }
    }

    let mut sorted_keys = keys.clone();
    sorted_keys.sort();
    let models: Vec<Value> = sorted_keys
        .into_iter()
        .map(|alias| {
            let (max_token, _provider) = if let Some(m) = models.get(&alias) {
                m.clone()
            } else {
                (-1, "ccproxy".to_string())
            };
            json!({
                "name": format!("models/{}", alias),
                "version": "1.0.0",
                "displayName": alias.clone(),
                "description": format!("ChatSpeed proxied model for {}", alias),
                "inputTokenLimit": max_token,
                "outputTokenLimit": max_token,
                "supportedGenerationMethods": [
                    "generateContent",
                    "streamGenerateContent"
                ],
                "temperature": 1.0,
                "topP": 1.0,
                "topK": 1,
                "powered_by": "chatspeed ccproxy",
            })
        })
        .collect();

    if models.is_empty() {
        log::info!("'chat_completion_proxy' configuration is empty or not found. Returning empty model list.");
    };

    let response = json!({
        "models": models,
    });

    Ok(Json(response))
}
