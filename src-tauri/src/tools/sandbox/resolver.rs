use super::types::{
    AgentSandboxConfig, HostFallbackReason, SandboxAvailabilityState, SandboxMountPlan,
    SandboxRuntime, SandboxRuntimePreference, SandboxRuntimeStatus, SandboxRuntimeStatusSummary,
    ShellExecutionBackendKind, ShellExecutionMode, ShellExecutionPlan, ShellExecutionPlanStatus,
    ShellExecutionRiskFloor,
};
use crate::tools::helper::{leading_command_index, shell_tokens, split_shell_command_segments};
use std::path::Path;

pub struct ShellExecutionResolver;

impl ShellExecutionResolver {
    pub fn resolve(
        tool_call_id: &str,
        command: &str,
        sandbox_config: Option<&AgentSandboxConfig>,
        runtime_status: &SandboxRuntimeStatusSummary,
        primary_root: Option<&Path>,
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

        let Some((profile_name, profile)) = choose_profile(config, command) else {
            return fallback_or_denied(
                tool_call_id,
                command,
                config,
                HostFallbackReason::ProfileUnavailable,
            );
        };

        let Some(primary_root) = primary_root else {
            return fallback_or_denied(
                tool_call_id,
                command,
                config,
                HostFallbackReason::ProfileUnavailable,
            );
        };
        if !profile.enabled || profile.image.trim().is_empty() {
            return fallback_or_denied(
                tool_call_id,
                command,
                config,
                HostFallbackReason::ProfileUnavailable,
            );
        }

        let preference =
            merge_runtime_preference(&config.runtime_preference, &profile.runtime_preference);
        let candidates = runtime_candidates(&preference);
        for runtime in candidates {
            if matches!(profile.network.mode, super::types::SandboxNetworkMode::Host)
                && runtime == SandboxRuntime::Msb
            {
                continue;
            }
            let status = runtime_status_for(&runtime, runtime_status);
            if !runtime_can_run_profile(status, &profile.image) {
                continue;
            }
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
            let configured_workdir = profile.workdir.as_deref().map(str::trim);
            let workdir = match configured_workdir {
                Some("") | None | Some("/workspace") => guest_path.clone(),
                Some(value) => value.to_string(),
            };
            return ShellExecutionPlan {
                tool_call_id: tool_call_id.to_string(),
                command: command.to_string(),
                backend: match runtime {
                    SandboxRuntime::Msb => ShellExecutionBackendKind::Msb,
                    SandboxRuntime::Docker => ShellExecutionBackendKind::Docker,
                },
                runtime: Some(runtime.clone()),
                profile: Some(profile_name),
                image: Some(profile.image.clone()),
                network: Some(profile.network.clone()),
                resources: Some(profile.resources.clone()),
                workspace_access: Some(profile.workspace_access.clone()),
                mounts: vec![SandboxMountPlan {
                    host_path,
                    guest_path,
                    access: profile.workspace_access.clone(),
                }],
                workdir: Some(workdir),
                fallback_reason: None,
                risk_floor: ShellExecutionRiskFloor::Normal,
                status: ShellExecutionPlanStatus::Ready,
            };
        }

        let reason = if runtime_candidates(&preference).iter().any(|runtime| {
            let status = runtime_status_for(runtime, runtime_status);
            matches!(
                status.state,
                SandboxAvailabilityState::Ready | SandboxAvailabilityState::ReadyMissingImage
            )
        }) {
            HostFallbackReason::MissingImage
        } else {
            HostFallbackReason::RuntimeUnavailable
        };
        fallback_or_denied(tool_call_id, command, config, reason)
    }
}

fn choose_profile<'a>(
    config: &'a AgentSandboxConfig,
    command: &str,
) -> Option<(String, &'a super::types::SandboxProfileConfig)> {
    let match_units = command_match_units(command);

    if let Some(default_profile) = config.profiles.get(&config.default_profile) {
        if default_profile.enabled
            && !is_common_profile(default_profile)
            && profile_matches_all_units(default_profile, &match_units)
        {
            return Some((config.default_profile.clone(), default_profile));
        }
    }

    if let Some((name, profile)) = config.profiles.iter().find(|(_, profile)| {
        profile.enabled
            && !is_common_profile(profile)
            && profile_matches_all_units(profile, &match_units)
    }) {
        return Some((name.clone(), profile));
    }

    // Profiles saved before commandPatterns was introduced keep the previous
    // capability inference until they are edited and migrated by the UI.
    let required = command_capabilities(command);
    if !required.is_empty() {
        if let Some((name, profile)) = config.profiles.iter().find(|(name, profile)| {
            profile.enabled
                && profile.command_patterns.is_empty()
                && !is_common_profile(profile)
                && profile_satisfies(name, profile, &required)
        }) {
            return Some((name.clone(), profile));
        }
    }

    if let Some(profile) = config.profiles.get(&config.default_profile) {
        if profile.enabled
            && (is_common_profile(profile)
                || profile_matches_all_units(profile, &match_units)
                || (profile.command_patterns.is_empty()
                    && profile_satisfies(&config.default_profile, profile, &required)))
        {
            return Some((config.default_profile.clone(), profile));
        }
    }

    config
        .profiles
        .iter()
        .find(|(_, profile)| profile.enabled && is_common_profile(profile))
        .map(|(name, profile)| (name.clone(), profile))
}

fn profile_matches_all_units(
    profile: &super::types::SandboxProfileConfig,
    match_units: &[String],
) -> bool {
    if match_units.is_empty() || profile.command_patterns.is_empty() {
        return false;
    }
    let patterns = profile
        .command_patterns
        .iter()
        .filter_map(|pattern| regex::Regex::new(pattern).ok())
        .collect::<Vec<_>>();
    !patterns.is_empty()
        && match_units
            .iter()
            .all(|unit| patterns.iter().any(|pattern| pattern.is_match(unit)))
}

fn is_common_profile(profile: &super::types::SandboxProfileConfig) -> bool {
    profile
        .command_patterns
        .iter()
        .any(|pattern| pattern.trim() == ".*")
        || (profile.command_patterns.is_empty()
            && profile.capabilities.len() == 1
            && profile.capabilities[0].eq_ignore_ascii_case("common"))
}

fn command_match_units(command: &str) -> Vec<String> {
    let mut units = Vec::new();
    for segment in split_shell_command_segments(command) {
        collect_segment_match_units(&segment, &mut units);
    }
    units
}

fn collect_segment_match_units(segment: &str, units: &mut Vec<String>) {
    let Some(tokens) = shell_tokens(segment) else {
        return;
    };
    let index = leading_command_index(&tokens);
    if index >= tokens.len() {
        return;
    }
    let executable = tokens[index].as_str();
    if matches!(executable, "cd" | "pushd" | "popd" | "tee" | "xargs") {
        return;
    }

    units.push(tokens[index..].join(" "));

    if matches!(executable, "npm" | "pnpm" | "yarn" | "npx" | "cargo") {
        let subcommands = tokens[index + 1..]
            .iter()
            .filter(|token| !token.starts_with('-'))
            .map(String::as_str)
            .collect::<Vec<_>>();
        if subcommands.first() == Some(&"tauri") {
            units.push(subcommands.join(" "));
        } else if subcommands.first() == Some(&"run") && subcommands.get(1) == Some(&"tauri") {
            units.push(subcommands[1..].join(" "));
        }
    }

    if matches!(executable, "sh" | "bash" | "zsh") {
        if let Some(script) = shell_c_script(&tokens, index + 1) {
            for nested in split_shell_command_segments(script) {
                collect_segment_match_units(&nested, units);
            }
        }
    }
}

fn command_capabilities(command: &str) -> Vec<&'static str> {
    let mut capabilities = Vec::new();
    for executable in command_executables(command) {
        let executable = executable
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(executable.as_str())
            .to_ascii_lowercase();
        for &capability in executable_capabilities(&executable) {
            if !capabilities.contains(&capability) {
                capabilities.push(capability);
            }
        }
    }
    capabilities
}

fn command_executables(command: &str) -> Vec<String> {
    let mut executables = Vec::new();
    for segment in split_shell_command_segments(command) {
        collect_segment_executables(&segment, &mut executables);
    }
    executables
}

fn collect_segment_executables(segment: &str, executables: &mut Vec<String>) {
    let Some(tokens) = shell_tokens(segment) else {
        return;
    };
    collect_tokens_executables(&tokens, executables);
}

fn collect_tokens_executables(tokens: &[String], executables: &mut Vec<String>) {
    if tokens.is_empty() {
        return;
    }
    let index = leading_command_index(tokens);
    if index >= tokens.len() {
        return;
    }
    let command = tokens[index].as_str();
    if matches!(command, "cd" | "pushd" | "popd" | "tee" | "xargs") {
        return;
    }
    push_executable(executables, command);

    if matches!(command, "npm" | "pnpm" | "yarn" | "npx" | "cargo") {
        let subcommand = tokens
            .iter()
            .skip(index + 1)
            .find(|token| !token.starts_with('-'))
            .map(String::as_str);
        if subcommand == Some("tauri")
            || (subcommand == Some("run")
                && tokens
                    .iter()
                    .skip(index + 2)
                    .find(|token| !token.starts_with('-'))
                    .is_some_and(|token| token == "tauri"))
        {
            push_executable(executables, "tauri");
        }
    }

    if matches!(command, "sh" | "bash" | "zsh") {
        if let Some(script) = shell_c_script(tokens, index + 1) {
            for nested in split_shell_command_segments(script) {
                collect_segment_executables(&nested, executables);
            }
        }
    }
}

fn shell_c_script(tokens: &[String], start: usize) -> Option<&str> {
    let mut index = start;
    while index < tokens.len() {
        match tokens[index].as_str() {
            "-c" => return tokens.get(index + 1).map(String::as_str),
            option if option.starts_with('-') => index += 1,
            _ => return None,
        }
    }
    None
}

fn push_executable(executables: &mut Vec<String>, executable: &str) {
    if !executables.iter().any(|existing| existing == executable) {
        executables.push(executable.to_string());
    }
}

fn executable_capabilities(executable: &str) -> &'static [&'static str] {
    match executable {
        "python" | "python3" | "pip" | "pip3" => &["python"],
        "node" | "npm" | "pnpm" | "yarn" | "npx" => &["node"],
        "cargo" | "rustc" | "rustup" | "rustfmt" | "rustdoc" | "cargo-fmt" | "cargo-clippy"
        | "clippy-driver" | "cargo-miri" | "miri" | "rust-analyzer" | "rust-gdb"
        | "rust-gdbgui" | "rust-lldb" => &["rust"],
        "tauri" => &["tauri"],
        "git" => &["git"],
        "go" => &["go"],
        "php" | "composer" => &["php"],
        _ => &[],
    }
}

fn profile_satisfies(
    profile_name: &str,
    profile: &super::types::SandboxProfileConfig,
    required: &[&'static str],
) -> bool {
    let capabilities = profile_capabilities(profile_name, profile);
    required
        .iter()
        .all(|capability| capabilities.iter().any(|available| available == capability))
}

fn profile_capabilities(
    profile_name: &str,
    profile: &super::types::SandboxProfileConfig,
) -> Vec<String> {
    if !profile.capabilities.is_empty() {
        return profile
            .capabilities
            .iter()
            .map(|capability| capability.trim().to_ascii_lowercase())
            .filter(|capability| !capability.is_empty())
            .collect();
    }

    let text = format!(
        "{} {}",
        profile_name.to_lowercase(),
        profile.image.to_lowercase()
    );
    let mut capabilities = vec!["common".to_string()];
    for (needle, capability) in [
        ("busybox", "common"),
        ("alpine", "common"),
        ("python", "python"),
        ("node", "node"),
        ("rust", "rust"),
        ("tauri", "tauri"),
        ("git", "git"),
        ("go", "go"),
        ("php", "php"),
    ] {
        if text.contains(needle) && !capabilities.iter().any(|item| item == capability) {
            capabilities.push(capability.to_string());
        }
    }
    capabilities
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

fn runtime_can_run_profile(status: &SandboxRuntimeStatus, image: &str) -> bool {
    matches!(
        status.state,
        SandboxAvailabilityState::Ready | SandboxAvailabilityState::ReadyMissingImage
    ) && runtime_has_image(status, image)
}

fn runtime_has_image(status: &SandboxRuntimeStatus, image: &str) -> bool {
    !status.images.is_empty() && status.images.iter().any(|available| available == image)
}

fn fallback_or_denied(
    tool_call_id: &str,
    command: &str,
    config: &AgentSandboxConfig,
    reason: HostFallbackReason,
) -> ShellExecutionPlan {
    if config.execution_mode == ShellExecutionMode::SandboxOnly {
        ShellExecutionPlan {
            tool_call_id: tool_call_id.to_string(),
            command: command.to_string(),
            backend: ShellExecutionBackendKind::Host,
            runtime: None,
            profile: None,
            image: None,
            network: None,
            resources: None,
            workspace_access: None,
            mounts: Vec::new(),
            workdir: None,
            fallback_reason: Some(reason),
            risk_floor: ShellExecutionRiskFloor::Normal,
            status: ShellExecutionPlanStatus::Denied,
        }
    } else {
        host_plan(tool_call_id, command, reason)
    }
}

fn host_plan(tool_call_id: &str, command: &str, reason: HostFallbackReason) -> ShellExecutionPlan {
    ShellExecutionPlan {
        tool_call_id: tool_call_id.to_string(),
        command: command.to_string(),
        backend: ShellExecutionBackendKind::Host,
        runtime: None,
        profile: None,
        image: None,
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
                enabled: true,
                capabilities: vec!["common".to_string()],
                command_patterns: vec![".*".to_string()],
                runtime_preference: SandboxRuntimePreference::Auto,
                image: "busybox:latest".to_string(),
                network: SandboxNetworkPolicy::default(),
                resources: SandboxResourceLimits::default(),
                workspace_access: WorkspaceAccess::ReadWrite,
                workdir: Some("/workspace".to_string()),
            },
        );
        AgentSandboxConfig {
            execution_mode: mode,
            runtime_preference: SandboxRuntimePreference::Auto,
            default_profile: "busybox".to_string(),
            profiles,
        }
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
        assert_eq!(plan.mounts.len(), 1);
        assert_eq!(plan.mounts[0].host_path, "/project");
        assert_eq!(plan.mounts[0].guest_path, "/project");
        assert_eq!(plan.workdir.as_deref(), Some("/project"));
        assert_eq!(plan.status, ShellExecutionPlanStatus::Ready);
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
        assert_eq!(plan.status, ShellExecutionPlanStatus::Ready);
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
        assert_eq!(plan.status, ShellExecutionPlanStatus::Ready);
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
    fn sandbox_only_denies_without_host_fallback() {
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
        capabilities: &[&str],
        command_patterns: &[&str],
    ) {
        config.profiles.insert(
            name.to_string(),
            SandboxProfileConfig {
                enabled: true,
                capabilities: capabilities.iter().map(ToString::to_string).collect(),
                command_patterns: command_patterns.iter().map(ToString::to_string).collect(),
                runtime_preference: SandboxRuntimePreference::Auto,
                image: image.to_string(),
                network: SandboxNetworkPolicy::default(),
                resources: SandboxResourceLimits::default(),
                workspace_access: WorkspaceAccess::ReadWrite,
                workdir: Some("/workspace".to_string()),
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
            &[],
            &[
                r"^(?:node|npm|pnpm|yarn|npx)(?:\s|$)",
                r"^(?:cargo|rustc|rustup)(?:\s|$)",
                r"^git(?:\s|$)",
            ],
        );
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
        assert_eq!(plan.mounts.len(), 1);
    }

    #[test]
    fn common_profile_is_the_final_sandbox_fallback() {
        let status = SandboxRuntimeStatusSummary {
            msb: ready_status(SandboxRuntime::Msb, vec!["busybox:latest"]),
            docker: ready_status(SandboxRuntime::Docker, vec![]),
        };
        let plan = ShellExecutionResolver::resolve(
            "tool-1",
            "pnpm build && cargo test",
            Some(&config(ShellExecutionMode::Auto)),
            &status,
            Some(Path::new("/project")),
        );
        assert_eq!(plan.backend, ShellExecutionBackendKind::Msb);
        assert_eq!(plan.profile.as_deref(), Some("busybox"));
        assert_eq!(plan.status, ShellExecutionPlanStatus::Ready);
    }

    #[test]
    fn unavailable_compound_profile_without_common_uses_host_fallback() {
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
        assert_eq!(plan.status, ShellExecutionPlanStatus::Ready);
        assert_eq!(plan.risk_floor, ShellExecutionRiskFloor::Normal);
        assert_eq!(
            plan.fallback_reason,
            Some(HostFallbackReason::ProfileUnavailable)
        );
    }

    #[test]
    fn command_analysis_uses_executable_tokens_not_quoted_text_or_paths() {
        assert_eq!(
            command_capabilities("echo 'python cargo node'"),
            Vec::<&str>::new()
        );
        assert_eq!(
            command_capabilities("cat ./fixtures/python-output.txt"),
            Vec::<&str>::new()
        );
        assert_eq!(
            command_capabilities("PYTHONPATH=src python -m pytest"),
            vec!["python"]
        );
        assert_eq!(
            command_capabilities("./scripts/cargo-wrapper --help"),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn command_analysis_handles_compound_pipes_redirects_and_shell_c() {
        assert_eq!(
            command_capabilities("cd app && pnpm build | tee out.log && cargo test > report.txt"),
            vec!["node", "rust"]
        );
        assert_eq!(
            command_capabilities("rustfmt --check src/lib.rs"),
            vec!["rust"]
        );
        assert_eq!(
            command_capabilities("cargo check && git diff src-tauri"),
            vec!["rust", "git"]
        );
        assert_eq!(
            command_capabilities("pnpm tauri build && cargo check && git diff"),
            vec!["node", "tauri", "rust", "git"]
        );
        assert_eq!(
            command_capabilities("bash -c 'python --version && node --version'"),
            vec!["python", "node"]
        );
        assert_eq!(
            command_capabilities("sh -lc 'echo cargo text only'"),
            Vec::<&str>::new()
        );
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
