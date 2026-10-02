//! Sandbox scheme Tauri commands.
//!
//! Sandbox scheme persistence is runtime-owned and reached through
//! `/control/v1/data-commands/*`; the runtime generates scheme and item ids.
//! Host runtime detection is a pure device probe and stays local, and the
//! `cs://sync-state` notification remains a local desktop concern.

use std::sync::Arc;

use tauri::{command, Emitter, State};

use crate::db::SandboxScheme;
use crate::runtime_client::RuntimeSupervisor;
use crate::tools::{
    AgentSandboxConfig, SandboxDetectorOptions, SandboxRuntimeDetector, SandboxRuntimeStatusSummary,
};

fn detect_sandbox_runtime_status(
    sandbox_config: Option<AgentSandboxConfig>,
) -> SandboxRuntimeStatusSummary {
    let required_images = sandbox_config
        .as_ref()
        .map(AgentSandboxConfig::required_images)
        .unwrap_or_default();
    SandboxRuntimeDetector::new(SandboxDetectorOptions {
        required_images,
        ..SandboxDetectorOptions::default()
    })
    .detect()
}

#[command]
pub async fn get_sandbox_runtime_status(
    sandbox_config: Option<AgentSandboxConfig>,
) -> Result<SandboxRuntimeStatusSummary, String> {
    Ok(detect_sandbox_runtime_status(sandbox_config))
}

#[command]
pub async fn refresh_sandbox_runtime_status(
    sandbox_config: Option<AgentSandboxConfig>,
) -> Result<SandboxRuntimeStatusSummary, String> {
    Ok(detect_sandbox_runtime_status(sandbox_config))
}

#[command]
pub async fn get_sandbox_scheme_runtime_status(
    config: crate::tools::SandboxSchemeConfig,
) -> Result<SandboxRuntimeStatusSummary, String> {
    let sandbox_config = AgentSandboxConfig {
        scheme_id: None,
        scheme_revision: None,
        execution_mode: crate::tools::ShellExecutionMode::Auto,
        runtime_preference: config.runtime_preference,
        profiles: config
            .profiles
            .into_iter()
            .map(|profile| (profile.id.clone(), profile))
            .collect(),
        host_rules: config.host_rules,
    };
    Ok(detect_sandbox_runtime_status(Some(sandbox_config)))
}

/// Lists all sandbox schemes.
#[command]
pub async fn get_sandbox_schemes(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Vec<SandboxScheme>, String> {
    crate::runtime_data::get_sandbox_schemes(supervisor.inner().as_ref()).await
}

fn emit_sandbox_schemes_changed(app: &tauri::AppHandle) {
    let _ = app.emit(
        "cs://sync-state",
        serde_json::json!({ "type": "sandbox_schemes_changed" }),
    );
}

/// Adds a sandbox scheme; the runtime assigns the scheme and item ids.
#[command]
pub async fn add_sandbox_scheme(
    app: tauri::AppHandle,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    scheme: SandboxScheme,
) -> Result<String, String> {
    let id = crate::runtime_data::add_sandbox_scheme(supervisor.inner().as_ref(), scheme).await?;
    emit_sandbox_schemes_changed(&app);
    Ok(id)
}

/// Updates a sandbox scheme.
#[command]
pub async fn update_sandbox_scheme(
    app: tauri::AppHandle,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    scheme: SandboxScheme,
) -> Result<(), String> {
    crate::runtime_data::update_sandbox_scheme(supervisor.inner().as_ref(), scheme).await?;
    emit_sandbox_schemes_changed(&app);
    Ok(())
}

/// Deletes a sandbox scheme.
#[command]
pub async fn delete_sandbox_scheme(
    app: tauri::AppHandle,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: String,
) -> Result<(), String> {
    crate::runtime_data::delete_sandbox_scheme(supervisor.inner().as_ref(), id).await?;
    emit_sandbox_schemes_changed(&app);
    Ok(())
}
