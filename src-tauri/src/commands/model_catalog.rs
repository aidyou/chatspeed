use serde_json::Value;
use tauri::State;

use crate::ai::model_catalog::{resolve_model_profile_from_catalog, ModelsDevProvider};
use crate::ai::model_catalog_updater::ModelsDevCatalogService;
use crate::ai::transport::resolve as resolve_transport;
use crate::ai::util::{
    get_family_from_model_id, is_function_call_supported, is_image_input_supported,
    is_reasoning_supported,
};
use crate::error::{AppError, Result};

/// Return only providers that can be represented by ChatSpeed's existing protocols.
/// This command never performs network I/O and never treats arbitrary catalog metadata as wire policy.
#[tauri::command]
pub fn list_models_dev_providers(
    service: State<'_, ModelsDevCatalogService>,
) -> Vec<ModelsDevProvider> {
    service
        .providers()
        .into_iter()
        .filter(|provider| {
            provider.api.as_deref().is_some_and(|api| {
                let api = api.trim();
                api.starts_with("https://") || api.starts_with("http://")
            })
        })
        .filter(|provider| {
            let npm = provider
                .npm
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let api = provider
                .api
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            npm.contains("anthropic")
                || npm.contains("openai")
                || api.contains("openai")
                || api.contains("anthropic")
                || api.contains("generativelanguage.googleapis.com")
                || api.contains("googleapis.com")
        })
        .filter(|provider| !provider.models.is_empty())
        .collect()
}

#[tauri::command]
pub fn resolve_model_profile(
    model_id: String,
    base_url: Option<String>,
    backend_protocol: Option<String>,
    metadata: Option<Value>,
    service: State<'_, ModelsDevCatalogService>,
) -> Result<crate::ai::model_catalog::ResolvedModelProfile> {
    let metadata_map = metadata.as_ref().and_then(|value| {
        value.as_object().map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect()
        })
    });
    let mut profile =
        resolve_model_profile_from_catalog(&service.snapshot(), &model_id).map_err(|error| {
            AppError::General {
                message: error.to_string(),
            }
        })?;
    let normalized_model_id = model_id.trim().to_ascii_lowercase();
    if profile.family.is_none() {
        profile.family = get_family_from_model_id(&normalized_model_id);
    }
    if profile.capabilities.reasoning.is_none() && is_reasoning_supported(&normalized_model_id) {
        profile.capabilities.reasoning = Some(true);
    }
    if profile.capabilities.function_call.is_none() {
        if is_function_call_supported(&normalized_model_id) {
            profile.capabilities.function_call = Some(true);
        }
    }
    if profile.capabilities.image_input.is_none() && is_image_input_supported(&normalized_model_id)
    {
        profile.capabilities.image_input = Some(true);
    }
    if let Some((adapter, transport_id)) = resolve_transport(
        &model_id,
        base_url.as_deref(),
        backend_protocol.as_deref(),
        metadata_map.as_ref(),
    )
    .map_err(|error| AppError::General {
        message: error.to_string(),
    })? {
        profile.thinking_adapter = Some(adapter);
        profile.matched_transport_id = Some(transport_id);
    }
    Ok(profile)
}
