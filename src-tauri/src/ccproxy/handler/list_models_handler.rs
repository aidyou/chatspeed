use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use rust_i18n::t;
use serde_json::{json, Value};
use warp::reply::Reply;

use crate::{
    ccproxy::{errors::ProxyResult, types::BackendModelTarget},
    db::MainStore,
};

// =================================================
//  chat completion proxy
// =================================================
/// Handles the `/v1/models` (list models) request.
/// Reads `chat_completion_proxy` from `MainStore`.
pub async fn handle_openai_list_models(
    main_store: Arc<Mutex<MainStore>>,
) -> ProxyResult<impl Reply> {
    let config: HashMap<String, Vec<BackendModelTarget>> = {
        if let Ok(store_guard) = main_store.lock() {
            let cfg: HashMap<String, Vec<BackendModelTarget>> =
                store_guard.get_config("chat_completion_proxy", HashMap::new());
            drop(store_guard);
            cfg
        } else {
            log::error!("{}", t!("db.failed_to_lock_main_store").to_string());
            HashMap::new()
        }
    };

    let keys = config.keys().cloned().collect::<Vec<String>>();
    let mut models: HashMap<String, (i32, String)> = HashMap::new();
    for (alias, target) in config.into_iter() {
        // If the alias has already been processed and added to `models`, skip it.
        // This prevents re-processing the same alias.
        if models.contains_key(&alias) {
            continue;
        }

        // Attempt to acquire a lock on the `main_store`.
        if let Ok(store) = main_store.lock() {
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
    }

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
            json!({
                "id": alias.clone(),
                "object": "model".to_string(),
                "created": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                "owned_by": provider,
                "max_tokens": max_token,
                "powered_by": "chatspeed ccproxy".to_string(),
            })
        })
        .collect();

    if models.is_empty() {
        log::info!("'chat_completion_proxy' configuration is empty or not found. Returning empty model list.");
    };

    let response = json!( {
        "object": "list".to_string(),
        "data": models,
    });

    Ok(warp::reply::json(&response))
}

/// List proxied models handler for ollama
/// GET `/api/tags`
pub async fn handle_ollama_tags(main_store: Arc<Mutex<MainStore>>) -> ProxyResult<impl Reply> {
    let config: HashMap<String, Vec<BackendModelTarget>> = {
        if let Ok(store_guard) = main_store.lock() {
            let cfg: HashMap<String, Vec<BackendModelTarget>> =
                store_guard.get_config("chat_completion_proxy", HashMap::new());
            drop(store_guard);
            cfg
        } else {
            log::error!("{}", t!("db.failed_to_lock_main_store").to_string());
            HashMap::new()
        }
    };

    let keys = config.keys().cloned().collect::<Vec<String>>();
    let mut models: HashMap<String, (i32, String)> = HashMap::new();
    for (alias, target) in config.into_iter() {
        // If the alias has already been processed and added to `models`, skip it.
        // This prevents re-processing the same alias.
        if models.contains_key(&alias) {
            continue;
        }

        // Attempt to acquire a lock on the `main_store`.
        if let Ok(store) = main_store.lock() {
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
            json!( {
                "name": alias.clone(),
                "model": alias.clone(),
                "modified_at": chrono::Utc::now().to_rfc3339(),
                "details": {
                    "parent_model": "",
                    "format": "gguf",
                    "family": provider,
                    "families":[provider]
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

    Ok(warp::reply::json(&response))
}
