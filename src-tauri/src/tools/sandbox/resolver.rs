use super::analyzer::analyze_shell_command;
use super::types::{
    AgentSandboxConfig, HostFallbackReason, SandboxAvailabilityState, SandboxMountPlan,
    SandboxRuntime, SandboxRuntimePreference, SandboxRuntimeStatus, SandboxRuntimeStatusSummary,
    ShellCommandAnalysis, ShellCommandStage, ShellExecutionBackendKind,
    ShellExecutionBackendOrigin, ShellExecutionMode, ShellExecutionPlan, ShellExecutionPlanStatus,
    ShellExecutionRiskFloor,
};
use std::path::{Path, PathBuf};

pub struct ShellExecutionResolver;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellSandboxMountContext {
    pub authorized_roots: Vec<PathBuf>,
    pub skill_roots: Vec<PathBuf>,
    pub writable_skill_roots: Vec<PathBuf>,
}

impl ShellExecutionResolver {
    pub fn complete_sandbox_mounts(
        mut plan: ShellExecutionPlan,
        context: &ShellSandboxMountContext,
    ) -> ShellExecutionPlan {
        if !matches!(
            plan.backend,
            ShellExecutionBackendKind::Msb | ShellExecutionBackendKind::Docker
        ) {
            return plan;
        }

        let primary_root = context.authorized_roots.first().cloned();
        let workspace_access = plan.workspace_access.clone().unwrap_or_default();
        let mut mounts = Vec::new();
        for root in &context.authorized_roots {
            push_mount(&mut mounts, root, workspace_access.clone());
        }
        for root in &context.skill_roots {
            let access = if context
                .writable_skill_roots
                .iter()
                .any(|writable| writable == root)
            {
                super::types::WorkspaceAccess::ReadWrite
            } else {
                super::types::WorkspaceAccess::ReadOnly
            };
            push_mount(&mut mounts, root, access);
        }
        let temp_root = crate::libs::ai_temp::ai_temp_physical_root_unchecked();
        push_mount(
            &mut mounts,
            &temp_root,
            super::types::WorkspaceAccess::ReadWrite,
        );
        plan.mounts = mounts;
        plan.workdir = primary_root
            .as_deref()
            .map(guest_path_for_host_path)
            .or_else(|| Some("/workspace".to_string()));
        plan
    }

    pub fn resolve(
        tool_call_id: &str,
        command: &str,
        sandbox_config: Option<&AgentSandboxConfig>,
        runtime_status: &SandboxRuntimeStatusSummary,
        primary_root: Option<&Path>,
    ) -> ShellExecutionPlan {
        let analysis = analyze_shell_command(command);
        let mut plan = Self::resolve_inner(
            tool_call_id,
            command,
            sandbox_config,
            runtime_status,
            primary_root,
            &analysis,
        );
        if let Some(config) = sandbox_config {
            plan.scheme_id = config.scheme_id.clone();
            plan.scheme_revision = config.scheme_revision.clone();
        }
        plan
    }

    fn resolve_inner(
        tool_call_id: &str,
        command: &str,
        sandbox_config: Option<&AgentSandboxConfig>,
        runtime_status: &SandboxRuntimeStatusSummary,
        primary_root: Option<&Path>,
        analysis: &ShellCommandAnalysis,
    ) -> ShellExecutionPlan {
        let Some(config) = sandbox_config else {
            return host_plan(
                tool_call_id,
                command,
                HostFallbackReason::SandboxConfigMissing,
            );
        };

        if config.execution_mode == ShellExecutionMode::HostOnly {
            return host_plan(tool_call_id, command, HostFallbackReason::HostOnlyMode);
        }

        if config.execution_mode == ShellExecutionMode::Auto {
            match choose_host_rule_for_stages(config, analysis) {
                Ok(Some(rule)) => return explicit_host_plan(tool_call_id, command, &rule.name),
                Ok(None) => {}
                Err(reason) => return denied_plan(tool_call_id, command, reason),
            }
        }

        let runnable_profile = match choose_runnable_profile(config, analysis, runtime_status) {
            Ok(Some(profile)) => Some(profile),
            Ok(None) => match choose_runnable_common_profile(config, runtime_status) {
                Ok(profile) => profile,
                Err(reason) => return denied_plan(tool_call_id, command, reason),
            },
            Err(reason) => return denied_plan(tool_call_id, command, reason),
        };
        let Some((profile_name, profile, runtime, instance_name)) = runnable_profile else {
            return denied_plan(
                tool_call_id,
                command,
                sandbox_unavailability_reason(config, analysis, runtime_status),
            );
        };

        let Some(primary_root) = primary_root else {
            return denied_plan(
                tool_call_id,
                command,
                HostFallbackReason::ProfileUnavailable,
            );
        };
        let host_path = primary_root.display().to_string();
        let guest_path = if primary_root.is_absolute()
            && primary_root
                .components()
                .next()
                .is_some_and(|component| matches!(component, std::path::Component::RootDir))
        {
            host_path.clone()
        } else {
            "/workspace".to_string()
        };
        let workdir = guest_path.clone();
        ShellExecutionPlan {
            tool_call_id: tool_call_id.to_string(),
            command: command.to_string(),
            scheme_id: None,
            scheme_revision: None,
            backend: match runtime {
                SandboxRuntime::Msb => ShellExecutionBackendKind::Msb,
                SandboxRuntime::Docker => ShellExecutionBackendKind::Docker,
            },
            backend_origin: ShellExecutionBackendOrigin::SandboxProfile,
            runtime: Some(runtime),
            profile: Some(profile_name),
            image: Some(profile.image.clone()),
            instance_name,
            network: Some(profile.network.clone()),
            resources: Some(profile.resources.clone()),
            workspace_access: Some(profile.workspace_access.clone()),
            mounts: vec![
                SandboxMountPlan {
                    host_path,
                    guest_path,
                    access: profile.workspace_access.clone(),
                },
                SandboxMountPlan {
                    host_path: crate::libs::ai_temp::ai_temp_physical_root_unchecked()
                        .to_string_lossy()
                        .to_string(),
                    guest_path: crate::libs::ai_temp::AI_TEMP_ROOT.to_string(),
                    access: super::types::WorkspaceAccess::ReadWrite,
                },
            ],
            workdir: Some(workdir),
            fallback_reason: None,
            risk_floor: ShellExecutionRiskFloor::Normal,
            status: ShellExecutionPlanStatus::Ready,
        }
    }
}

fn push_mount(
    mounts: &mut Vec<SandboxMountPlan>,
    host_root: &Path,
    access: super::types::WorkspaceAccess,
) {
    if !host_root.exists() {
        return;
    }
    let host_path = host_root.to_string_lossy().to_string();
    let guest_path = if host_root == crate::libs::ai_temp::ai_temp_physical_root_unchecked() {
        crate::libs::ai_temp::AI_TEMP_ROOT.to_string()
    } else {
        guest_path_for_host_path(host_root)
    };
    if let Some(existing) = mounts
        .iter_mut()
        .find(|mount| mount.guest_path == guest_path)
    {
        if access == super::types::WorkspaceAccess::ReadWrite {
            existing.access = access;
        }
        return;
    }
    mounts.push(SandboxMountPlan {
        host_path,
        guest_path,
        access,
    });
}

#[cfg(not(windows))]
fn guest_path_for_host_path(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

#[cfg(windows)]
fn guest_path_for_host_path(path: &Path) -> String {
    use std::path::Component;

    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return "/workspace".to_string();
    };
    let drive = match prefix.kind() {
        std::path::Prefix::Disk(letter) | std::path::Prefix::VerbatimDisk(letter) => {
            (letter as char).to_ascii_lowercase().to_string()
        }
        _ => return "/workspace".to_string(),
    };
    let suffix = components
        .filter_map(|component| match component {
            Component::Normal(segment) => segment.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    if suffix.is_empty() {
        format!("/mnt/{drive}")
    } else {
        format!("/mnt/{drive}/{suffix}")
    }
}

fn host_rule_matches_stage(
    rule: &super::types::HostCommandRule,
    stage: &ShellCommandStage,
) -> bool {
    rule.enabled
        && !rule.command_patterns.is_empty()
        && rule
            .command_patterns
            .iter()
            .filter_map(|pattern| regex::Regex::new(pattern).ok())
            .any(|pattern| pattern.is_match(&stage.normalized_command))
}

fn choose_host_rule_for_stages<'a>(
    config: &'a AgentSandboxConfig,
    analysis: &ShellCommandAnalysis,
) -> Result<Option<&'a super::types::HostCommandRule>, HostFallbackReason> {
    if analysis.stages.is_empty() {
        return Ok(None);
    }

    let mut candidates = config
        .host_rules
        .iter()
        .filter(|rule| {
            analysis
                .stages
                .iter()
                .all(|stage| host_rule_matches_stage(rule, stage))
        })
        .collect::<Vec<_>>();
    if let Some(priority) = candidates.iter().map(|rule| rule.priority).max() {
        candidates.retain(|rule| rule.priority == priority);
        if candidates.len() != 1 {
            return Err(HostFallbackReason::AmbiguousRoute);
        }
        return Ok(candidates.pop());
    }

    if config.host_rules.iter().any(|rule| {
        analysis
            .stages
            .iter()
            .any(|stage| host_rule_matches_stage(rule, stage))
    }) {
        return Err(HostFallbackReason::MixedBackendCommand);
    }

    Ok(None)
}

fn choose_runnable_profile<'a>(
    config: &'a AgentSandboxConfig,
    analysis: &ShellCommandAnalysis,
    runtime_status: &SandboxRuntimeStatusSummary,
) -> Result<
    Option<(
        String,
        &'a super::types::SandboxProfileConfig,
        SandboxRuntime,
        Option<String>,
    )>,
    HostFallbackReason,
> {
    let mut candidates = config
        .profiles
        .iter()
        .filter(|(_, profile)| {
            profile.enabled
                && !profile.image.trim().is_empty()
                && !super::types::is_common_profile(profile)
                && profile_covers_analysis(profile, analysis)
        })
        .filter_map(|(name, profile)| {
            let preference =
                merge_runtime_preference(&config.runtime_preference, &profile.runtime_preference);
            choose_runtime_execution_target(profile, &preference, runtime_status)
                .map(|(runtime, instance_name)| (name.clone(), profile, runtime, instance_name))
        })
        .collect::<Vec<_>>();
    let Some(priority) = candidates
        .iter()
        .map(|(_, profile, _, _)| profile.priority)
        .max()
    else {
        return Ok(None);
    };
    candidates.retain(|(_, profile, _, _)| profile.priority == priority);
    if candidates.len() == 1 {
        return Ok(candidates.pop());
    }

    // Explicit user rule: for equally capable profiles at the same priority, prefer the
    // uniquely smallest persisted image because it is cheaper to start and transfer.
    let image_sizes = candidates
        .iter()
        .map(|(_, profile, _, _)| profile.image_size_bytes)
        .collect::<Option<Vec<_>>>()
        .ok_or(HostFallbackReason::AmbiguousRoute)?;
    let image_size_bytes = image_sizes
        .into_iter()
        .min()
        .ok_or(HostFallbackReason::AmbiguousRoute)?;
    candidates.retain(|(_, profile, _, _)| profile.image_size_bytes == Some(image_size_bytes));
    if candidates.len() != 1 {
        return Err(HostFallbackReason::AmbiguousRoute);
    }
    Ok(candidates.pop())
}

fn choose_runnable_common_profile<'a>(
    config: &'a AgentSandboxConfig,
    runtime_status: &SandboxRuntimeStatusSummary,
) -> Result<
    Option<(
        String,
        &'a super::types::SandboxProfileConfig,
        SandboxRuntime,
        Option<String>,
    )>,
    HostFallbackReason,
> {
    super::types::enabled_common_profile(config.profiles.values())
        .map_err(|_| HostFallbackReason::AmbiguousRoute)?;
    let Some((profile_name, profile)) = config
        .profiles
        .iter()
        .find(|(_, profile)| profile.enabled && super::types::is_common_profile(profile))
    else {
        return Ok(None);
    };
    let preference =
        merge_runtime_preference(&config.runtime_preference, &profile.runtime_preference);
    Ok(
        choose_runtime_execution_target(profile, &preference, runtime_status).map(
            |(runtime, instance_name)| (profile_name.clone(), profile, runtime, instance_name),
        ),
    )
}

fn sandbox_unavailability_reason(
    config: &AgentSandboxConfig,
    analysis: &ShellCommandAnalysis,
    runtime_status: &SandboxRuntimeStatusSummary,
) -> HostFallbackReason {
    let matching_profiles = config.profiles.iter().filter(|(_, profile)| {
        profile.enabled
            && !profile.image.trim().is_empty()
            && (config.execution_mode != ShellExecutionMode::Auto
                || !profile
                    .command_patterns
                    .iter()
                    .any(|pattern| super::types::is_catch_all_command_pattern(pattern)))
            && profile_covers_analysis(profile, analysis)
    });
    if matching_profiles.count() == 0
        && super::types::enabled_common_profile(config.profiles.values())
            .ok()
            .flatten()
            .is_none()
    {
        return HostFallbackReason::ProfileUnavailable;
    }
    if runtime_status
        .msb
        .state
        .eq(&SandboxAvailabilityState::Ready)
        || runtime_status
            .msb
            .state
            .eq(&SandboxAvailabilityState::ReadyMissingImage)
        || runtime_status
            .docker
            .state
            .eq(&SandboxAvailabilityState::Ready)
        || runtime_status
            .docker
            .state
            .eq(&SandboxAvailabilityState::ReadyMissingImage)
    {
        HostFallbackReason::MissingImage
    } else {
        HostFallbackReason::RuntimeUnavailable
    }
}

fn profile_covers_analysis(
    profile: &super::types::SandboxProfileConfig,
    analysis: &ShellCommandAnalysis,
) -> bool {
    if analysis.stages.is_empty() || profile.command_patterns.is_empty() {
        return false;
    }

    let patterns = profile
        .command_patterns
        .iter()
        .filter_map(|pattern| regex::Regex::new(pattern).ok())
        .collect::<Vec<_>>();
    !patterns.is_empty()
        && analysis.stages.iter().all(|stage| {
            patterns
                .iter()
                .any(|pattern| pattern.is_match(&stage.normalized_command))
        })
}

fn merge_runtime_preference(
    config_preference: &SandboxRuntimePreference,
    profile_preference: &SandboxRuntimePreference,
) -> SandboxRuntimePreference {
    if *profile_preference == SandboxRuntimePreference::Auto {
        config_preference.clone()
    } else {
        profile_preference.clone()
    }
}

fn runtime_candidates(preference: &SandboxRuntimePreference) -> Vec<SandboxRuntime> {
    match preference {
        SandboxRuntimePreference::Auto => vec![SandboxRuntime::Msb, SandboxRuntime::Docker],
        SandboxRuntimePreference::Msb => vec![SandboxRuntime::Msb],
        SandboxRuntimePreference::Docker => vec![SandboxRuntime::Docker],
    }
}

fn runtime_status_for<'a>(
    runtime: &SandboxRuntime,
    status: &'a SandboxRuntimeStatusSummary,
) -> &'a SandboxRuntimeStatus {
    match runtime {
        SandboxRuntime::Msb => &status.msb,
        SandboxRuntime::Docker => &status.docker,
    }
}

fn choose_runtime_execution_target(
    profile: &super::types::SandboxProfileConfig,
    preference: &SandboxRuntimePreference,
    runtime_status: &SandboxRuntimeStatusSummary,
) -> Option<(SandboxRuntime, Option<String>)> {
    let mut image_fallback = None;
    for runtime in runtime_candidates(preference) {
        if matches!(profile.network.mode, super::types::SandboxNetworkMode::Host)
            && runtime == SandboxRuntime::Msb
        {
            continue;
        }
        match runtime_execution_target(runtime_status_for(&runtime, runtime_status), profile) {
            Some(Some(instance_name)) => return Some((runtime, Some(instance_name))),
            Some(None) if image_fallback.is_none() => image_fallback = Some((runtime, None)),
            _ => {}
        }
    }
    image_fallback
}

fn runtime_execution_target(
    status: &SandboxRuntimeStatus,
    profile: &super::types::SandboxProfileConfig,
) -> Option<Option<String>> {
    if !matches!(
        status.state,
        SandboxAvailabilityState::Ready | SandboxAvailabilityState::ReadyMissingImage
    ) {
        return None;
    }
    if let Some(instance_name) = profile
        .instance_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        if status
            .running_instances
            .iter()
            .any(|available| available == instance_name)
        {
            return Some(Some(instance_name.to_string()));
        }
    }
    runtime_has_image(status, &profile.image).then_some(None)
}

fn runtime_has_image(status: &SandboxRuntimeStatus, image: &str) -> bool {
    !status.images.is_empty() && status.images.iter().any(|available| available == image)
}

fn denied_plan(
    tool_call_id: &str,
    command: &str,
    reason: HostFallbackReason,
) -> ShellExecutionPlan {
    let mut plan = host_plan(tool_call_id, command, reason);
    plan.status = ShellExecutionPlanStatus::Denied;
    plan
}

fn explicit_host_plan(tool_call_id: &str, command: &str, rule_name: &str) -> ShellExecutionPlan {
    let mut plan = host_plan(tool_call_id, command, HostFallbackReason::HostOnlyMode);
    plan.backend_origin = ShellExecutionBackendOrigin::ExplicitHostRule;
    plan.profile = Some(rule_name.to_string());
    plan
}

fn host_plan(tool_call_id: &str, command: &str, reason: HostFallbackReason) -> ShellExecutionPlan {
    ShellExecutionPlan {
        tool_call_id: tool_call_id.to_string(),
        command: command.to_string(),
        scheme_id: None,
        scheme_revision: None,
        backend: ShellExecutionBackendKind::Host,
        backend_origin: ShellExecutionBackendOrigin::HostOnly,
        runtime: None,
        profile: None,
        image: None,
        instance_name: None,
        network: None,
        resources: None,
        workspace_access: None,
        mounts: Vec::new(),
        workdir: None,
        fallback_reason: Some(reason),
        risk_floor: ShellExecutionRiskFloor::Normal,
        status: ShellExecutionPlanStatus::Ready,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{
        SandboxNetworkPolicy, SandboxProfileConfig, SandboxResourceLimits, WorkspaceAccess,
    };
    use std::collections::BTreeMap;

    fn ready_status(runtime: SandboxRuntime, images: Vec<&str>) -> SandboxRuntimeStatus {
        runtime_status(runtime, SandboxAvailabilityState::Ready, images, Vec::new())
    }

    fn ready_missing_image_status(
        runtime: SandboxRuntime,
        images: Vec<&str>,
        missing_images: Vec<&str>,
    ) -> SandboxRuntimeStatus {
        runtime_status(
            runtime,
            SandboxAvailabilityState::ReadyMissingImage,
            images,
            missing_images,
        )
    }

    fn runtime_status(
        runtime: SandboxRuntime,
        state: SandboxAvailabilityState,
        images: Vec<&str>,
        missing_images: Vec<&str>,
    ) -> SandboxRuntimeStatus {
        SandboxRuntimeStatus {
            runtime,
            state,
            executable: Some("runtime".to_string()),
            version: Some("1.0.0".to_string()),
            reason_code: Some("ready".to_string()),
            reason: Some("ready".to_string()),
            images: images.into_iter().map(ToString::to_string).collect(),
            image_sizes: BTreeMap::new(),
            running_instances: Vec::new(),
            missing_images: missing_images
                .into_iter()
                .map(ToString::to_string)
                .collect(),
            checked_at_ms: None,
        }
    }

    fn config(mode: ShellExecutionMode) -> AgentSandboxConfig {
        let mut profiles = BTreeMap::new();
        profiles.insert(
            "busybox".to_string(),
            SandboxProfileConfig {
                id: "busybox".to_string(),
                name: "busybox".to_string(),
                enabled: true,
                priority: 0,
                command_patterns: vec![".*".to_string()],
                runtime_preference: SandboxRuntimePreference::Auto,
                image: "busybox:latest".to_string(),
                instance_name: None,
                image_size_bytes: Some(1),
                network: SandboxNetworkPolicy::default(),
                resources: SandboxResourceLimits::default(),
                workspace_access: WorkspaceAccess::ReadWrite,
            },
        );
        AgentSandboxConfig {
            scheme_id: None,
            scheme_revision: None,
            execution_mode: mode,
            runtime_preference: SandboxRuntimePreference::Auto,
            profiles,
            host_rules: Vec::new(),
        }
    }

    #[test]
    fn available_instance_is_preferred_and_missing_instance_uses_image() {
        let mut config = config(ShellExecutionMode::Auto);
        let profile = config.profiles.get_mut("busybox").expect("common profile");
        profile.instance_name = Some("dev-container".to_string());

        let mut msb = ready_status(SandboxRuntime::Msb, vec!["busybox:latest"]);
        msb.running_instances = vec!["dev-container".to_string()];
        let status = SandboxRuntimeStatusSummary {
            msb,
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-instance",
            "echo hi",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.backend, ShellExecutionBackendKind::Msb);
        assert_eq!(plan.instance_name.as_deref(), Some("dev-container"));
        assert_eq!(plan.image.as_deref(), Some("busybox:latest"));

        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["busybox:latest"]),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-image-fallback",
            "echo hi",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.backend, ShellExecutionBackendKind::Msb);
        assert_eq!(plan.instance_name, None);
        assert_eq!(plan.image.as_deref(), Some("busybox:latest"));
    }

    #[test]
    fn auto_runtime_prefers_docker_instance_over_msb_image_fallback() {
        let mut config = config(ShellExecutionMode::Auto);
        config
            .profiles
            .get_mut("busybox")
            .expect("common profile")
            .instance_name = Some("dev-container".to_string());
        let mut docker = ready_status(SandboxRuntime::Docker, vec![]);
        docker.running_instances = vec!["dev-container".to_string()];
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["busybox:latest"]),
            docker,
        };

        let plan = ShellExecutionResolver::resolve(
            "tool-docker-instance",
            "echo hi",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.backend, ShellExecutionBackendKind::Docker);
        assert_eq!(plan.instance_name.as_deref(), Some("dev-container"));
    }

    #[test]
    fn auto_runtime_prefers_msb_instance_over_docker_image_fallback() {
        let mut config = config(ShellExecutionMode::Auto);
        config
            .profiles
            .get_mut("busybox")
            .expect("common profile")
            .instance_name = Some("dev-container".to_string());
        let mut msb = ready_status(SandboxRuntime::Msb, vec![]);
        msb.running_instances = vec!["dev-container".to_string()];
        let status = SandboxRuntimeStatusSummary {
            msb,
            docker: ready_status(SandboxRuntime::Docker, vec!["busybox:latest"]),
        };

        let plan = ShellExecutionResolver::resolve(
            "tool-msb-instance",
            "echo hi",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.backend, ShellExecutionBackendKind::Msb);
        assert_eq!(plan.instance_name.as_deref(), Some("dev-container"));
    }

    #[test]
    fn complete_sandbox_mounts_preserves_authorized_paths_and_maps_stable_tmp() {
        let workspace = tempfile::tempdir().unwrap();
        let secondary = tempfile::tempdir().unwrap();
        let user_skills = tempfile::tempdir().unwrap();
        let builtin_skills = tempfile::tempdir().unwrap();
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["busybox:latest"]),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::complete_sandbox_mounts(
            ShellExecutionResolver::resolve(
                "tool-1",
                "echo hi",
                Some(&config(ShellExecutionMode::Auto)),
                &status,
                Some(workspace.path()),
            ),
            &ShellSandboxMountContext {
                authorized_roots: vec![
                    workspace.path().to_path_buf(),
                    secondary.path().to_path_buf(),
                ],
                skill_roots: vec![
                    user_skills.path().to_path_buf(),
                    builtin_skills.path().to_path_buf(),
                ],
                writable_skill_roots: vec![user_skills.path().to_path_buf()],
            },
        );
        assert_eq!(plan.workdir.as_deref(), workspace.path().to_str());
        assert!(plan.mounts.iter().any(|mount| {
            mount.host_path == workspace.path().to_string_lossy()
                && mount.guest_path == workspace.path().to_string_lossy()
                && mount.access == WorkspaceAccess::ReadWrite
        }));
        assert!(plan.mounts.iter().any(|mount| {
            mount.host_path == secondary.path().to_string_lossy()
                && mount.guest_path == secondary.path().to_string_lossy()
                && mount.access == WorkspaceAccess::ReadWrite
        }));
        assert!(plan.mounts.iter().any(|mount| {
            mount.host_path == user_skills.path().to_string_lossy()
                && mount.guest_path == user_skills.path().to_string_lossy()
                && mount.access == WorkspaceAccess::ReadWrite
        }));
        assert!(plan.mounts.iter().any(|mount| {
            mount.host_path == builtin_skills.path().to_string_lossy()
                && mount.guest_path == builtin_skills.path().to_string_lossy()
                && mount.access == WorkspaceAccess::ReadOnly
        }));
        let physical_temp_root = crate::libs::ai_temp::ai_temp_physical_root_unchecked();
        #[cfg(target_os = "macos")]
        assert_eq!(physical_temp_root, Path::new("/private/tmp/chatspeed"));
        assert!(plan.mounts.iter().any(|mount| {
            mount.host_path == physical_temp_root.to_string_lossy()
                && mount.guest_path == crate::libs::ai_temp::AI_TEMP_ROOT
                && mount.access == WorkspaceAccess::ReadWrite
        }));
    }

    #[test]
    fn resolves_ready_msb_sandbox_plan() {
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["busybox:latest"]),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "echo hi && pwd",
            Some(&config(ShellExecutionMode::Auto)),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.backend, ShellExecutionBackendKind::Msb);
        assert_eq!(plan.mounts.len(), 2);
        assert_eq!(plan.mounts[0].host_path, "/project");
        assert_eq!(plan.mounts[0].guest_path, "/project");
        assert_eq!(plan.workdir.as_deref(), Some("/project"));
        assert_eq!(plan.status, ShellExecutionPlanStatus::Ready);
    }

    #[test]
    fn resolves_scheme_snapshot_identity_into_execution_plan() {
        let mut config = config(ShellExecutionMode::Auto);
        config.scheme_id = Some("scheme-1".to_string());
        config.scheme_revision = Some("2026-08-04T00:00:00Z".to_string());
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["busybox:latest"]),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "echo hi",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.scheme_id.as_deref(), Some("scheme-1"));
        assert_eq!(
            plan.scheme_revision.as_deref(),
            Some("2026-08-04T00:00:00Z")
        );
    }

    #[test]
    fn ready_runtime_with_empty_image_list_does_not_execute_profile() {
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec![]),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "echo hi",
            Some(&config(ShellExecutionMode::Auto)),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.backend, ShellExecutionBackendKind::Host);
        assert_eq!(plan.status, ShellExecutionPlanStatus::Denied);
        assert_eq!(plan.risk_floor, ShellExecutionRiskFloor::Normal);
        assert_eq!(plan.fallback_reason, Some(HostFallbackReason::MissingImage));
    }

    #[test]
    fn ready_runtime_missing_selected_image_uses_missing_image_fallback() {
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["alpine:latest"]),
            docker: ready_status(SandboxRuntime::Docker, vec!["alpine:latest"]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "echo hi",
            Some(&config(ShellExecutionMode::Auto)),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.backend, ShellExecutionBackendKind::Host);
        assert_eq!(plan.status, ShellExecutionPlanStatus::Denied);
        assert_eq!(plan.risk_floor, ShellExecutionRiskFloor::Normal);
        assert_eq!(plan.fallback_reason, Some(HostFallbackReason::MissingImage));
    }

    #[test]
    fn ready_missing_image_runtime_executes_when_selected_profile_image_exists() {
        let status = SandboxRuntimeStatusSummary {
            msb: ready_missing_image_status(
                SandboxRuntime::Msb,
                vec!["busybox:latest"],
                vec!["python:3.12-slim"],
            ),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "echo hi",
            Some(&config(ShellExecutionMode::Auto)),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.backend, ShellExecutionBackendKind::Msb);
        assert_eq!(plan.status, ShellExecutionPlanStatus::Ready);
        assert_eq!(plan.image.as_deref(), Some("busybox:latest"));
        assert_eq!(plan.fallback_reason, None);
    }

    #[test]
    fn sandbox_only_denies_when_sandbox_is_unavailable() {
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["alpine:latest"]),
            docker: ready_status(SandboxRuntime::Docker, vec!["alpine:latest"]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "echo hi",
            Some(&config(ShellExecutionMode::SandboxOnly)),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.backend, ShellExecutionBackendKind::Host);
        assert_eq!(plan.status, ShellExecutionPlanStatus::Denied);
        assert_eq!(plan.risk_floor, ShellExecutionRiskFloor::Normal);
    }

    fn add_profile(
        config: &mut AgentSandboxConfig,
        name: &str,
        image: &str,
        command_patterns: &[&str],
    ) {
        assert!(!command_patterns.is_empty());
        let command_patterns = command_patterns.iter().map(ToString::to_string).collect();
        config.profiles.insert(
            name.to_string(),
            SandboxProfileConfig {
                id: name.to_string(),
                name: name.to_string(),
                enabled: true,
                priority: 0,
                command_patterns,
                runtime_preference: SandboxRuntimePreference::Auto,
                image: image.to_string(),
                instance_name: None,
                image_size_bytes: Some(1),
                network: SandboxNetworkPolicy::default(),
                resources: SandboxResourceLimits::default(),
                workspace_access: WorkspaceAccess::ReadWrite,
            },
        );
    }

    #[test]
    fn compound_command_selects_one_superset_profile_without_splitting() {
        let mut config = config(ShellExecutionMode::Auto);
        add_profile(
            &mut config,
            "node-rust",
            "node-rust:latest",
            &[
                r"^(?:node|npm|pnpm|yarn|npx)(?:\s|$)",
                r"^(?:cargo|rustc|rustup)(?:\s|$)",
                r"^git(?:\s|$)",
            ],
        );
        config.profiles.get_mut("node-rust").unwrap().priority = 10;
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(
                SandboxRuntime::Msb,
                vec!["busybox:latest", "node-rust:latest"],
            ),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let command = "cd src-tauri && pnpm build && cargo test --lib | tee /tmp/out && git diff";
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            command,
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.backend, ShellExecutionBackendKind::Msb);
        assert_eq!(plan.profile.as_deref(), Some("node-rust"));
        assert_eq!(plan.command, command);
        assert_eq!(plan.mounts.len(), 2);
    }

    #[test]
    fn compound_navigation_command_matches_git_and_cargo_rules() {
        let mut config = config(ShellExecutionMode::Auto);
        config.profiles.clear();
        add_profile(
            &mut config,
            "git-cargo",
            "git-cargo:latest",
            &[r"^git log(?:\s|$)", r"^cargo check(?:\s|$)"],
        );
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["git-cargo:latest"]),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let command = "git log -n 3 && cd src-tauri/src && cargo check";
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            command,
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.status, ShellExecutionPlanStatus::Ready);
        assert_eq!(plan.profile.as_deref(), Some("git-cargo"));
    }

    #[test]
    fn full_command_host_rule_beats_higher_priority_partial_rule() {
        let mut config = config(ShellExecutionMode::Auto);
        config.profiles.clear();
        config.host_rules.extend([
            super::super::types::HostCommandRule {
                id: "full-host".to_string(),
                name: "Full Host".to_string(),
                enabled: true,
                priority: 5,
                command_patterns: vec![
                    r"^git log(?:\s|$)".to_string(),
                    r"^cargo check(?:\s|$)".to_string(),
                ],
            },
            super::super::types::HostCommandRule {
                id: "partial-host".to_string(),
                name: "Partial Host".to_string(),
                enabled: true,
                priority: 10,
                command_patterns: vec![r"^git log(?:\s|$)".to_string()],
            },
        ]);
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec![]),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "git log -n 3 && cd src-tauri/src && cargo check",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.status, ShellExecutionPlanStatus::Ready);
        assert_eq!(plan.backend, ShellExecutionBackendKind::Host);
        assert_eq!(
            plan.backend_origin,
            ShellExecutionBackendOrigin::ExplicitHostRule
        );
    }

    #[test]
    fn sandbox_priority_is_compared_with_the_full_command_host_rule() {
        let mut config = config(ShellExecutionMode::Auto);
        config.profiles.clear();
        add_profile(
            &mut config,
            "git-cargo",
            "git-cargo:latest",
            &[r"^git log(?:\s|$)", r"^cargo check(?:\s|$)"],
        );
        config.profiles.get_mut("git-cargo").unwrap().priority = 20;
        config.host_rules.extend([
            super::super::types::HostCommandRule {
                id: "full-host".to_string(),
                name: "Full Host".to_string(),
                enabled: true,
                priority: 10,
                command_patterns: vec![
                    r"^git log(?:\s|$)".to_string(),
                    r"^cargo check(?:\s|$)".to_string(),
                ],
            },
            super::super::types::HostCommandRule {
                id: "partial-host".to_string(),
                name: "Partial Host".to_string(),
                enabled: true,
                priority: 100,
                command_patterns: vec![r"^git log(?:\s|$)".to_string()],
            },
        ]);
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["git-cargo:latest"]),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "git log -n 3 && cd src-tauri/src && cargo check",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.status, ShellExecutionPlanStatus::Ready);
        assert_eq!(plan.backend, ShellExecutionBackendKind::Host);
        assert_eq!(plan.profile.as_deref(), Some("Full Host"));
        assert_eq!(
            plan.backend_origin,
            ShellExecutionBackendOrigin::ExplicitHostRule
        );
    }

    #[test]
    fn host_rules_only_apply_in_auto_mode() {
        let mut config = config(ShellExecutionMode::SandboxOnly);
        config
            .host_rules
            .push(super::super::types::HostCommandRule {
                id: "tauri-host".to_string(),
                name: "Tauri Host".to_string(),
                enabled: true,
                priority: 100,
                command_patterns: vec![],
            });
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["busybox:latest"]),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "pnpm test",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.status, ShellExecutionPlanStatus::Ready);
        assert_eq!(plan.profile.as_deref(), Some("busybox"));
    }

    #[test]
    fn runnable_lower_priority_profile_beats_unavailable_higher_priority_profile() {
        let mut config = config(ShellExecutionMode::Auto);
        config.profiles.remove("busybox");
        add_profile(
            &mut config,
            "node-low",
            "node-low:latest",
            &[r"^pnpm(?:\s|$)"],
        );
        add_profile(
            &mut config,
            "node-high-unavailable",
            "node-high:latest",
            &[r"^pnpm(?:\s|$)"],
        );
        config.profiles.get_mut("node-low").unwrap().priority = 10;
        config
            .profiles
            .get_mut("node-high-unavailable")
            .unwrap()
            .priority = 20;
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["node-low:latest"]),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "pnpm test",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.status, ShellExecutionPlanStatus::Ready);
        assert_eq!(plan.profile.as_deref(), Some("node-low"));
        assert_eq!(
            plan.backend_origin,
            ShellExecutionBackendOrigin::SandboxProfile
        );
    }

    #[test]
    fn highest_priority_profile_wins_without_map_order_dependence() {
        let mut config = config(ShellExecutionMode::Auto);
        add_profile(
            &mut config,
            "node-low",
            "node-low:latest",
            &[r"^pnpm(?:\s|$)"],
        );
        add_profile(
            &mut config,
            "node-high",
            "node-high:latest",
            &[r"^pnpm(?:\s|$)"],
        );
        config.profiles.get_mut("node-low").unwrap().priority = 10;
        config.profiles.get_mut("node-high").unwrap().priority = 20;
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(
                SandboxRuntime::Msb,
                vec!["busybox:latest", "node-low:latest", "node-high:latest"],
            ),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "pnpm test",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.profile.as_deref(), Some("node-high"));
    }

    #[test]
    fn equal_priority_profiles_fail_closed() {
        let mut config = config(ShellExecutionMode::Auto);
        add_profile(&mut config, "node-a", "node-a:latest", &[r"^pnpm(?:\s|$)"]);
        add_profile(&mut config, "node-b", "node-b:latest", &[r"^pnpm(?:\s|$)"]);
        config.profiles.remove("busybox");
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["node-a:latest", "node-b:latest"]),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "pnpm test",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.status, ShellExecutionPlanStatus::Denied);
        assert_eq!(
            plan.fallback_reason,
            Some(HostFallbackReason::AmbiguousRoute)
        );
    }

    #[test]
    fn equal_priority_host_rules_fail_closed() {
        let mut config = config(ShellExecutionMode::Auto);
        config.profiles.remove("busybox");
        for id in ["host-a", "host-b"] {
            config
                .host_rules
                .push(super::super::types::HostCommandRule {
                    id: id.to_string(),
                    name: id.to_string(),
                    enabled: true,
                    priority: 10,
                    command_patterns: vec![r"^pnpm(?:\s|$)".to_string()],
                });
        }
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec![]),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "pnpm test",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.status, ShellExecutionPlanStatus::Denied);
        assert_eq!(
            plan.fallback_reason,
            Some(HostFallbackReason::AmbiguousRoute)
        );
    }

    #[test]
    fn stage_specific_host_rule_rejects_mixed_backend_command() {
        let mut config = config(ShellExecutionMode::Auto);
        add_profile(
            &mut config,
            "node-rust-tauri",
            "node-rust-tauri:latest",
            &[
                r"^(?:pnpm|npm|yarn|npx)(?:\s+run)?\s+tauri(?:\s|$)",
                r"^cargo(?:\s|$)",
            ],
        );
        config
            .host_rules
            .push(super::super::types::HostCommandRule {
                id: "tauri-host".to_string(),
                name: "Tauri Host".to_string(),
                enabled: true,
                priority: 100,
                command_patterns: vec![
                    r"^(?:pnpm|npm|yarn|npx)(?:\s+run)?\s+tauri(?:\s|$)".to_string()
                ],
            });
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(
                SandboxRuntime::Msb,
                vec!["busybox:latest", "node-rust-tauri:latest"],
            ),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "pnpm tauri build && cargo test",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.status, ShellExecutionPlanStatus::Denied);
        assert_eq!(
            plan.fallback_reason,
            Some(HostFallbackReason::MixedBackendCommand)
        );
    }

    #[test]
    fn explicit_host_rule_carries_distinct_backend_origin() {
        let mut config = config(ShellExecutionMode::Auto);
        config
            .host_rules
            .push(super::super::types::HostCommandRule {
                id: "build-host".to_string(),
                name: "Build Host".to_string(),
                enabled: true,
                priority: 100,
                command_patterns: vec![r"^pnpm(?:\s|$)".to_string(), r"^cargo(?:\s|$)".to_string()],
            });
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["busybox:latest"]),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "pnpm test && cargo test",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.backend, ShellExecutionBackendKind::Host);
        assert_eq!(
            plan.backend_origin,
            ShellExecutionBackendOrigin::ExplicitHostRule
        );
        assert_eq!(plan.status, ShellExecutionPlanStatus::Ready);
    }

    #[test]
    fn auto_unmatched_command_uses_common_profile() {
        let config = config(ShellExecutionMode::Auto);
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["busybox:latest"]),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "git status --short --branch",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.backend, ShellExecutionBackendKind::Msb);
        assert_eq!(plan.profile.as_deref(), Some("busybox"));
        assert_eq!(plan.status, ShellExecutionPlanStatus::Ready);
    }

    #[test]
    fn sandbox_only_profile_pattern_covers_matching_commands() {
        let config = config(ShellExecutionMode::SandboxOnly);
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["busybox:latest"]),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "git status --short --branch",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.backend, ShellExecutionBackendKind::Msb);
        assert_eq!(plan.profile.as_deref(), Some("busybox"));
        assert_eq!(plan.status, ShellExecutionPlanStatus::Ready);
    }

    #[test]
    fn profile_command_patterns_route_without_name_inference() {
        let mut config = config(ShellExecutionMode::Auto);
        config.profiles.clear();
        add_profile(&mut config, "git", "git:latest", &[r"^git(?:\s|$)"]);
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["git:latest"]),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "git status --short --branch",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.status, ShellExecutionPlanStatus::Ready);
        assert_eq!(plan.profile.as_deref(), Some("git"));
    }

    #[test]
    fn same_priority_profiles_choose_the_smaller_persisted_image() {
        let mut config = config(ShellExecutionMode::Auto);
        config.profiles.remove("busybox");
        add_profile(
            &mut config,
            "python-small",
            "python-small:latest",
            &[r"^python(?:\s|$)"],
        );
        add_profile(
            &mut config,
            "python-large",
            "python-large:latest",
            &[r"^python(?:\s|$)"],
        );
        config
            .profiles
            .get_mut("python-small")
            .unwrap()
            .image_size_bytes = Some(50);
        config
            .profiles
            .get_mut("python-large")
            .unwrap()
            .image_size_bytes = Some(100);
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(
                SandboxRuntime::Msb,
                vec!["python-small:latest", "python-large:latest"],
            ),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "python -c 'print(1)'",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.profile.as_deref(), Some("python-small"));
        assert_eq!(plan.status, ShellExecutionPlanStatus::Ready);
    }

    #[test]
    fn same_priority_and_image_size_profiles_fail_closed() {
        let mut config = config(ShellExecutionMode::Auto);
        config.profiles.clear();
        for name in ["python-a", "python-b"] {
            add_profile(
                &mut config,
                name,
                &format!("{name}:latest"),
                &[r"^python(?:\s|$)"],
            );
            config.profiles.get_mut(name).unwrap().image_size_bytes = Some(50);
        }
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(
                SandboxRuntime::Msb,
                vec!["python-a:latest", "python-b:latest"],
            ),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "python -c 'print(1)'",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.status, ShellExecutionPlanStatus::Denied);
        assert_eq!(
            plan.fallback_reason,
            Some(HostFallbackReason::AmbiguousRoute)
        );
    }

    #[test]
    fn unavailable_compound_profile_without_common_is_denied() {
        let mut config = config(ShellExecutionMode::Auto);
        config.profiles.remove("busybox");
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["busybox:latest"]),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "pnpm build && cargo test",
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.backend, ShellExecutionBackendKind::Host);
        assert_eq!(plan.status, ShellExecutionPlanStatus::Denied);
        assert_eq!(plan.risk_floor, ShellExecutionRiskFloor::Normal);
        assert_eq!(
            plan.fallback_reason,
            Some(HostFallbackReason::ProfileUnavailable)
        );
    }

    #[test]
    fn compound_command_requires_one_profile_to_cover_every_stage() {
        let mut config = config(ShellExecutionMode::Auto);
        config.profiles.clear();
        add_profile(
            &mut config,
            "cargo-only",
            "cargo:latest",
            &[r"^cargo(?:\s|$)"],
        );
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["cargo:latest"]),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let command = "cargo check && git diff src-tauri";
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            command,
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.status, ShellExecutionPlanStatus::Denied);
        assert_eq!(
            plan.fallback_reason,
            Some(HostFallbackReason::ProfileUnavailable)
        );

        config
            .profiles
            .get_mut("cargo-only")
            .unwrap()
            .command_patterns
            .push(r"^git(?:\s|$)".to_string());
        let plan = ShellExecutionResolver::resolve(
            "tool-2",
            command,
            Some(&config),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.status, ShellExecutionPlanStatus::Ready);
        assert_eq!(plan.profile.as_deref(), Some("cargo-only"));
    }

    #[test]
    fn host_only_uses_host_without_overriding_shell_policy() {
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["busybox:latest"]),
            docker: ready_status(SandboxRuntime::Docker, vec!["busybox:latest"]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "echo hi",
            Some(&config(ShellExecutionMode::HostOnly)),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.backend, ShellExecutionBackendKind::Host);
        assert_eq!(plan.risk_floor, ShellExecutionRiskFloor::Normal);
        assert_eq!(plan.status, ShellExecutionPlanStatus::Ready);
    }
}
