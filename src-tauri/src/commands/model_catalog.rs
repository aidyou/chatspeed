use std::str::FromStr;

use serde_json::Value;
use tauri::State;

use crate::ai::model_catalog::{
    resolve_model_profile_from_catalog_with_context, ModelsDevPresetProvider,
};
use crate::ai::model_catalog_updater::ModelsDevCatalogService;
use crate::ai::traits::chat::ModelDetails;
use crate::ai::transport::resolve as resolve_transport;
use crate::ai::util::{
    get_family_from_model_id, is_function_call_supported, is_image_input_supported,
    is_reasoning_supported,
};
use crate::ccproxy::ChatProtocol;
use crate::error::{AppError, Result};

/// Return the generated provider presets embedded with the application.
#[tauri::command]
pub fn list_models_dev_providers(
    service: State<'_, ModelsDevCatalogService>,
) -> Vec<ModelsDevPresetProvider> {
    service.preset_providers().providers.clone()
}

/// Return catalog models for a provider when its live list-models endpoint is unavailable.
#[tauri::command]
pub fn list_models_dev_provider_models(
    provider_id: String,
    service: State<'_, ModelsDevCatalogService>,
) -> Vec<ModelDetails> {
    service
        .snapshot()
        .providers
        .get(&provider_id)
        .map(|provider| {
            provider
                .models
                .values()
                .filter_map(|model| {
                    let protocol = ChatProtocol::from_str("openai").ok()?;
                    Some(ModelDetails {
                        id: model.id.clone(),
                        name: model.name.clone(),
                        protocol,
                        max_input_tokens: model
                            .limit
                            .as_ref()
                            .and_then(|limit| limit.input.map(|value| value as u32)),
                        max_output_tokens: model
                            .limit
                            .as_ref()
                            .and_then(|limit| limit.output.map(|value| value as u32)),
                        description: None,
                        last_updated: model
                            .extra
                            .get("last_updated")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        family: model.family.clone(),
                        reasoning: model.reasoning,
                        function_call: model.tool_call,
                        image_input: model.modalities.as_ref().map(|modalities| {
                            modalities.input.iter().any(|input| input == "image")
                        }),
                        recommended_temperature: None,
                        metadata: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
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
    let provider_id = metadata
        .as_ref()
        .and_then(|value| value.get("modelsDevProviderId").and_then(Value::as_str));
    let mut profile = resolve_model_profile_from_catalog_with_context(
        &service.snapshot(),
        &model_id,
        provider_id,
        base_url.as_deref(),
    )
    .map_err(|error| AppError::General {
        message: error.to_string(),
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

#[cfg(test)]
mod tests {
    use crate::ai::model_catalog::models_dev_preset_providers;

    #[test]
    fn embedded_provider_presets_include_openrouter() {
        let providers = models_dev_preset_providers().expect("embedded provider presets");
        let openrouter = providers
            .providers
            .iter()
            .find(|provider| provider.id == "openrouter")
            .expect("OpenRouter provider preset");

        assert_eq!(openrouter.protocol.as_deref(), Some("openai"));
        assert_eq!(
            openrouter.api.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
    }
}
