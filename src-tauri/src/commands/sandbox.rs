use tauri::command;

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
