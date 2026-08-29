use serde_json::Value;

use crate::ai::model_catalog::{resolve_model_profile as resolve_profile, resolve_transport};
use crate::error::{AppError, Result};

/// Resolve static catalog facts for a model without duplicating catalog matching in the frontend.
#[tauri::command]
pub fn resolve_model_profile(
    model_id: String,
    base_url: Option<String>,
    backend_protocol: Option<String>,
    metadata: Option<Value>,
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
    let mut profile = resolve_profile(&model_id).map_err(|error| AppError::General {
        message: error.to_string(),
    })?;
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
