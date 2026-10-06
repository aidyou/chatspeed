//! Models.dev catalog Tauri commands.
//!
//! The catalog snapshot, its loading and its refresh are owned by the runtime.
//! These wrappers only translate the Tauri wire into
//! `/control/v1/data-commands/*` calls through the [`RuntimeSupervisor`]; the
//! runtime returns the historical camelCase payloads unchanged, so the frontend
//! keeps calling the same commands with the same shapes.

use std::sync::Arc;

use serde_json::Value;
use tauri::{command, State};

use crate::ai::model_catalog::{ModelsDevPresetProvider, ResolvedModelProfile};
use crate::ai::traits::chat::ModelDetails;
use crate::error::AppError;
use crate::runtime_client::RuntimeSupervisor;
use chatspeed_contracts::ResolveModelProfileRequest;

/// Return the generated provider presets embedded with the application.
#[command]
pub async fn list_models_dev_providers(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Vec<ModelsDevPresetProvider>, String> {
    crate::runtime_data::list_models_dev_providers(supervisor.inner().as_ref()).await
}

/// Return catalog models for a provider when its live list-models endpoint is unavailable.
#[command]
pub async fn list_models_dev_provider_models(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    provider_id: String,
) -> Result<Vec<ModelDetails>, String> {
    crate::runtime_data::list_models_dev_provider_models(supervisor.inner().as_ref(), provider_id)
        .await
}

/// Resolve the catalog profile for a model against the runtime snapshot.
#[command]
pub async fn resolve_model_profile(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    model_id: String,
    base_url: Option<String>,
    backend_protocol: Option<String>,
    metadata: Option<Value>,
) -> Result<ResolvedModelProfile, AppError> {
    crate::runtime_data::resolve_model_profile(
        supervisor.inner().as_ref(),
        ResolveModelProfileRequest {
            model_id,
            base_url,
            backend_protocol,
            metadata,
        },
    )
    .await
    .map_err(|message| AppError::General { message })
}
