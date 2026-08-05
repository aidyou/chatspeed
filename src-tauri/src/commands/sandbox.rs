use std::sync::Arc;

use tauri::{command, State};

use crate::{
    db::{MainStore, SandboxScheme},
    tools::{
        AgentSandboxConfig, SandboxDetectorOptions, SandboxRuntimeDetector,
        SandboxRuntimeStatusSummary,
    },
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
        default_profile: String::new(),
        profiles: config
            .profiles
            .into_iter()
            .map(|profile| (profile.id.clone(), profile))
            .collect(),
        host_rules: config.host_rules,
    };
    Ok(detect_sandbox_runtime_status(Some(sandbox_config)))
}

#[command]
pub async fn get_sandbox_schemes(
    state: State<'_, Arc<MainStore>>,
) -> Result<Vec<SandboxScheme>, String> {
    state
        .get_all_sandbox_schemes()
        .map_err(|error| error.to_string())
}

fn assign_missing_profile_ids(
    scheme: &mut SandboxScheme,
    tsid_generator: &crate::libs::tsid::TsidGenerator,
) -> Result<(), String> {
    for profile in &mut scheme.config.profiles {
        if profile.id.trim().is_empty() {
            profile.id = tsid_generator
                .generate()
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[command]
pub async fn add_sandbox_scheme(
    state: State<'_, Arc<MainStore>>,
    tsid_generator: State<'_, Arc<crate::libs::tsid::TsidGenerator>>,
    mut scheme: SandboxScheme,
) -> Result<String, String> {
    scheme.id = tsid_generator
        .generate()
        .map_err(|error| error.to_string())?;
    assign_missing_profile_ids(&mut scheme, &tsid_generator)?;
    state
        .add_sandbox_scheme(&scheme)
        .map_err(|error| error.to_string())
}

#[command]
pub async fn update_sandbox_scheme(
    state: State<'_, Arc<MainStore>>,
    tsid_generator: State<'_, Arc<crate::libs::tsid::TsidGenerator>>,
    mut scheme: SandboxScheme,
) -> Result<(), String> {
    assign_missing_profile_ids(&mut scheme, &tsid_generator)?;
    state
        .update_sandbox_scheme(&scheme)
        .map_err(|error| error.to_string())
}

#[command]
pub async fn delete_sandbox_scheme(
    state: State<'_, Arc<MainStore>>,
    id: String,
) -> Result<(), String> {
    state
        .delete_sandbox_scheme(&id)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{
        SandboxNetworkPolicy, SandboxProfileConfig, SandboxSchemeConfig, WorkspaceAccess,
    };

    #[test]
    fn assigns_tsid_to_new_profile_without_client_supplied_id() {
        let generator = crate::libs::tsid::TsidGenerator::new(1).expect("create TSID generator");
        let mut scheme = SandboxScheme {
            id: "scheme".to_string(),
            name: "Scheme".to_string(),
            description: String::new(),
            config: SandboxSchemeConfig {
                runtime_preference: Default::default(),
                profiles: vec![SandboxProfileConfig {
                    id: String::new(),
                    name: "Bash".to_string(),
                    enabled: true,
                    priority: 0,
                    capabilities: vec!["bash".to_string()],
                    command_patterns: vec!["^bash(?:\\s|$)".to_string()],
                    runtime_preference: Default::default(),
                    image: "bash:latest".to_string(),
                    image_size_bytes: None,
                    network: SandboxNetworkPolicy::default(),
                    resources: Default::default(),
                    workspace_access: WorkspaceAccess::ReadWrite,
                }],
                host_rules: vec![],
            },
            disabled: false,
            created_at: None,
            updated_at: None,
        };

        assign_missing_profile_ids(&mut scheme, &generator).expect("assign profile ID");

        assert_eq!(scheme.config.profiles[0].id.len(), 13);
    }
}
