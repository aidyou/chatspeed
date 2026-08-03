use super::types::{
    SandboxFailure, SandboxFailureReason, SandboxNetworkMode, ShellExecutionBackendKind,
    ShellExecutionPlan, WorkspaceAccess,
};
use crate::tools::ToolError;
use std::process::Stdio;
use tokio::process::Command;

const SANDBOX_INSTANCE_NAME_PREFIX: &str = "chatspeed-shell";

pub fn sandbox_command_for_plan(
    plan: &ShellExecutionPlan,
    original_command: &str,
) -> Result<Option<Command>, ToolError> {
    let Some(argv) = sandbox_argv_for_plan(plan, original_command)? else {
        return Ok(None);
    };
    let mut iter = argv.into_iter();
    let program = iter
        .next()
        .ok_or_else(|| ToolError::ExecutionFailed("sandbox command argv is empty".to_string()))?;
    let mut command = Command::new(program);
    command.args(iter);
    configure_child_stdio(&mut command);
    Ok(Some(command))
}

pub fn sandbox_instance_name(plan: &ShellExecutionPlan) -> String {
    let raw = format!(
        "{}-{}",
        SANDBOX_INSTANCE_NAME_PREFIX,
        sanitize_instance_component(&plan.tool_call_id)
    );
    raw.chars().take(96).collect()
}

fn sandbox_failure(
    plan: &ShellExecutionPlan,
    reason: SandboxFailureReason,
    message: impl Into<String>,
) -> ToolError {
    ToolError::SandboxFailure(SandboxFailure::from_plan(plan, reason, message))
}

fn sanitize_instance_component(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            sanitized.push(ch.to_ascii_lowercase());
        } else {
            sanitized.push('-');
        }
    }
    let sanitized = sanitized.trim_matches('-');
    if sanitized.is_empty() {
        "tool".to_string()
    } else {
        sanitized.to_string()
    }
}

pub fn sandbox_cleanup_command_for_plan(plan: &ShellExecutionPlan) -> Option<Command> {
    let argv = sandbox_cleanup_argv_for_plan(plan)?;
    let mut iter = argv.into_iter();
    let program = iter.next()?;
    let mut command = Command::new(program);
    command.args(iter);
    command.stdout(Stdio::null()).stderr(Stdio::null());
    Some(command)
}

pub fn sandbox_cleanup_argv_for_plan(plan: &ShellExecutionPlan) -> Option<Vec<String>> {
    let name = sandbox_instance_name(plan);
    match plan.backend {
        ShellExecutionBackendKind::Msb => Some(vec![
            "msb".to_string(),
            "remove".to_string(),
            "--force".to_string(),
            "--quiet".to_string(),
            name,
        ]),
        ShellExecutionBackendKind::Docker => Some(vec![
            "docker".to_string(),
            "rm".to_string(),
            "-f".to_string(),
            name,
        ]),
        ShellExecutionBackendKind::Host => None,
    }
}

pub fn sandbox_argv_for_plan(
    plan: &ShellExecutionPlan,
    original_command: &str,
) -> Result<Option<Vec<String>>, ToolError> {
    match plan.backend {
        ShellExecutionBackendKind::Msb => build_msb_argv(plan, original_command).map(Some),
        ShellExecutionBackendKind::Docker => build_docker_argv(plan, original_command).map(Some),
        ShellExecutionBackendKind::Host => Ok(None),
    }
}

pub fn effective_timeout_ms(plan: &ShellExecutionPlan, requested_timeout_ms: Option<u64>) -> u64 {
    requested_timeout_ms
        .or_else(|| {
            plan.resources
                .as_ref()
                .and_then(|resources| resources.timeout_ms)
        })
        .unwrap_or(120_000)
        .min(600_000)
}

fn build_msb_argv(
    plan: &ShellExecutionPlan,
    original_command: &str,
) -> Result<Vec<String>, ToolError> {
    let image = plan.image.as_deref().ok_or_else(|| {
        sandbox_failure(
            plan,
            SandboxFailureReason::InvalidPlan,
            "sandbox plan missing Microsandbox image",
        )
    })?;
    let mut argv = vec![
        "msb".to_string(),
        "run".to_string(),
        "--quiet".to_string(),
        "--no-tty".to_string(),
        "--name".to_string(),
        sandbox_instance_name(plan),
    ];
    match plan.network.as_ref().map(|network| &network.mode) {
        Some(SandboxNetworkMode::None) | None => argv.push("--no-net".to_string()),
        Some(SandboxNetworkMode::Public) => {}
        Some(SandboxNetworkMode::Host) => {
            return Err(sandbox_failure(
                plan,
                SandboxFailureReason::UnsupportedNetwork,
                "Microsandbox host networking is not supported; select Docker for host-accessible ports",
            ));
        }
        Some(SandboxNetworkMode::Allowlist) => {
            return Err(sandbox_failure(
                plan,
                SandboxFailureReason::UnsupportedNetwork,
                "Microsandbox domain allowlist networking is not supported by the first sandbox runner",
            ));
        }
    }
    if let Some(resources) = &plan.resources {
        if let Some(cpus) = resources.cpus {
            argv.extend(["--cpus".to_string(), cpus.to_string()]);
        }
        if let Some(memory_mb) = resources.memory_mb {
            argv.extend(["--memory".to_string(), format!("{}M", memory_mb)]);
        }
    }
    for mount in &plan.mounts {
        let readonly = if mount.access == WorkspaceAccess::ReadOnly {
            ":ro"
        } else {
            ""
        };
        argv.extend([
            "--volume".to_string(),
            format!("{}:{}{}", mount.host_path, mount.guest_path, readonly),
        ]);
    }
    if let Some(workdir) = plan.workdir.as_deref() {
        argv.extend(["--workdir".to_string(), workdir.to_string()]);
    }
    argv.extend([
        image.to_string(),
        "--".to_string(),
        "sh".to_string(),
        "-lc".to_string(),
        original_command.to_string(),
    ]);
    Ok(argv)
}

fn build_docker_argv(
    plan: &ShellExecutionPlan,
    original_command: &str,
) -> Result<Vec<String>, ToolError> {
    let image = plan.image.as_deref().ok_or_else(|| {
        sandbox_failure(
            plan,
            SandboxFailureReason::InvalidPlan,
            "sandbox plan missing Docker image",
        )
    })?;
    if matches!(
        plan.network.as_ref().map(|network| &network.mode),
        Some(SandboxNetworkMode::Allowlist)
    ) {
        return Err(sandbox_failure(
            plan,
            SandboxFailureReason::UnsupportedNetwork,
            "Docker domain allowlist networking is not supported by the first sandbox runner",
        ));
    }

    let mut argv = vec![
        "docker".to_string(),
        "run".to_string(),
        "--rm".to_string(),
        "--name".to_string(),
        sandbox_instance_name(plan),
    ];
    match plan.network.as_ref().map(|network| &network.mode) {
        Some(SandboxNetworkMode::None) | None => {
            argv.extend(["--network".to_string(), "none".to_string()]);
        }
        Some(SandboxNetworkMode::Public) => {}
        Some(SandboxNetworkMode::Host) => {
            argv.extend(["--network".to_string(), "host".to_string()]);
        }
        Some(SandboxNetworkMode::Allowlist) => unreachable!(),
    }
    for mount in &plan.mounts {
        let readonly = if mount.access == WorkspaceAccess::ReadOnly {
            ",readonly"
        } else {
            ""
        };
        argv.extend([
            "--mount".to_string(),
            format!(
                "type=bind,src={},dst={}{}",
                mount.host_path, mount.guest_path, readonly
            ),
        ]);
    }
    if let Some(resources) = &plan.resources {
        if let Some(memory_mb) = resources.memory_mb {
            argv.extend(["--memory".to_string(), format!("{}m", memory_mb)]);
        }
        if let Some(cpus) = resources.cpus {
            argv.extend(["--cpus".to_string(), cpus.to_string()]);
        }
    }
    if let Some(workdir) = plan.workdir.as_deref() {
        argv.extend(["--workdir".to_string(), workdir.to_string()]);
    }
    argv.extend([
        image.to_string(),
        "sh".to_string(),
        "-lc".to_string(),
        original_command.to_string(),
    ]);
    Ok(argv)
}

fn configure_child_stdio(command: &mut Command) {
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{
        SandboxMountPlan, SandboxNetworkPolicy, SandboxResourceLimits, SandboxRuntime,
        ShellExecutionPlanStatus, ShellExecutionRiskFloor,
    };

    fn plan(backend: ShellExecutionBackendKind) -> ShellExecutionPlan {
        ShellExecutionPlan {
            tool_call_id: "tool-1".to_string(),
            command: "echo hi && pwd".to_string(),
            backend,
            runtime: Some(SandboxRuntime::Docker),
            profile: Some("busybox".to_string()),
            image: Some("busybox:latest".to_string()),
            network: Some(SandboxNetworkPolicy::default()),
            resources: Some(SandboxResourceLimits {
                cpus: Some(1),
                memory_mb: Some(128),
                timeout_ms: None,
            }),
            workspace_access: Some(WorkspaceAccess::ReadOnly),
            mounts: vec![SandboxMountPlan {
                host_path: "/project".to_string(),
                guest_path: "/workspace".to_string(),
                access: WorkspaceAccess::ReadOnly,
            }],
            workdir: Some("/workspace".to_string()),
            fallback_reason: None,
            risk_floor: ShellExecutionRiskFloor::Normal,
            status: ShellExecutionPlanStatus::Ready,
        }
    }

    #[test]
    fn msb_args_include_readonly_mount_and_limits() {
        let argv = sandbox_argv_for_plan(&plan(ShellExecutionBackendKind::Msb), "echo hi")
            .unwrap()
            .unwrap();
        assert_eq!(argv[0], "msb");
        assert!(argv
            .windows(2)
            .any(|pair| pair[0] == "--volume" && pair[1] == "/project:/workspace:ro"));
        assert!(argv
            .windows(2)
            .any(|pair| pair[0] == "--cpus" && pair[1] == "1"));
        assert!(argv
            .windows(2)
            .any(|pair| pair[0] == "--memory" && pair[1] == "128M"));
        assert!(!argv.iter().any(|arg| arg == "--timeout"));
        assert!(argv
            .windows(3)
            .any(|window| window[0] == "sh" && window[1] == "-lc" && window[2] == "echo hi"));
        assert_eq!(
            effective_timeout_ms(&plan(ShellExecutionBackendKind::Msb), None),
            120_000
        );
    }

    #[test]
    fn docker_args_include_network_mount_limits_and_no_unsafe_flags() {
        let argv = sandbox_argv_for_plan(&plan(ShellExecutionBackendKind::Docker), "echo hi")
            .unwrap()
            .unwrap();
        assert_eq!(argv[0], "docker");
        assert!(argv
            .windows(2)
            .any(|pair| pair[0] == "--network" && pair[1] == "none"));
        assert!(argv.windows(2).any(|pair| pair[0] == "--mount"
            && pair[1] == "type=bind,src=/project,dst=/workspace,readonly"));
        assert!(argv
            .windows(2)
            .any(|pair| pair[0] == "--memory" && pair[1] == "128m"));
        assert!(argv
            .windows(2)
            .any(|pair| pair[0] == "--cpus" && pair[1] == "1"));
        assert!(argv
            .windows(3)
            .any(|window| window[0] == "sh" && window[1] == "-lc" && window[2] == "echo hi"));
        assert!(!argv.iter().any(|arg| arg == "--privileged"));
        assert!(!argv.iter().any(|arg| arg.contains("docker.sock")));
        assert!(!argv
            .iter()
            .any(|arg| arg.contains("/.ssh") || arg.contains("/home/")));
    }

    #[test]
    fn docker_host_network_exposes_container_ports_to_the_host() {
        let mut plan = plan(ShellExecutionBackendKind::Docker);
        plan.network = Some(SandboxNetworkPolicy {
            mode: SandboxNetworkMode::Host,
            allowlist: Vec::new(),
        });
        let argv = sandbox_argv_for_plan(&plan, "python -m http.server 8000")
            .unwrap()
            .unwrap();
        assert!(argv
            .windows(2)
            .any(|pair| pair[0] == "--network" && pair[1] == "host"));
    }

    #[test]
    fn msb_host_network_is_rejected_not_widened() {
        let mut plan = plan(ShellExecutionBackendKind::Msb);
        plan.network = Some(SandboxNetworkPolicy {
            mode: SandboxNetworkMode::Host,
            allowlist: Vec::new(),
        });
        assert!(matches!(
            sandbox_command_for_plan(&plan, "python -m http.server 8000"),
            Err(ToolError::SandboxFailure(failure))
                if failure.reason == SandboxFailureReason::UnsupportedNetwork
                    && failure.backend == ShellExecutionBackendKind::Msb
        ));
    }

    #[test]
    fn effective_timeout_uses_ai_override_or_profile_default() {
        let mut plan = plan(ShellExecutionBackendKind::Msb);
        let mut resources = plan.resources.clone().unwrap();
        resources.timeout_ms = Some(30_000);
        plan.resources = Some(resources);
        assert_eq!(effective_timeout_ms(&plan, None), 30_000);
        assert_eq!(effective_timeout_ms(&plan, Some(10_000)), 10_000);
        assert_eq!(effective_timeout_ms(&plan, Some(90_000)), 90_000);
        assert_eq!(effective_timeout_ms(&plan, Some(900_000)), 600_000);
    }

    #[test]
    fn msb_allowlist_is_rejected_not_widened() {
        let mut plan = plan(ShellExecutionBackendKind::Msb);
        plan.network = Some(SandboxNetworkPolicy {
            mode: SandboxNetworkMode::Allowlist,
            allowlist: vec!["example.com".to_string()],
        });
        assert!(matches!(
            sandbox_command_for_plan(&plan, "echo hi"),
            Err(ToolError::SandboxFailure(failure))
                if failure.reason == SandboxFailureReason::UnsupportedNetwork
                    && failure.backend == ShellExecutionBackendKind::Msb
        ));
        assert!(matches!(
            sandbox_argv_for_plan(&plan, "echo hi"),
            Err(ToolError::SandboxFailure(failure))
                if failure.reason == SandboxFailureReason::UnsupportedNetwork
                    && failure.backend == ShellExecutionBackendKind::Msb
        ));
    }

    #[test]
    fn docker_allowlist_is_rejected_not_widened() {
        let mut plan = plan(ShellExecutionBackendKind::Docker);
        plan.network = Some(SandboxNetworkPolicy {
            mode: SandboxNetworkMode::Allowlist,
            allowlist: vec!["example.com".to_string()],
        });
        assert!(matches!(
            sandbox_command_for_plan(&plan, "echo hi"),
            Err(ToolError::SandboxFailure(failure))
                if failure.reason == SandboxFailureReason::UnsupportedNetwork
                    && failure.backend == ShellExecutionBackendKind::Docker
        ));
    }

    #[test]
    fn sandbox_names_are_stable_and_cleanup_argv_targets_instance() {
        let mut docker_plan = plan(ShellExecutionBackendKind::Docker);
        docker_plan.tool_call_id = "Tool Call/ABC 123".to_string();
        let name = sandbox_instance_name(&docker_plan);
        assert_eq!(name, "chatspeed-shell-tool-call-abc-123");
        let run_argv = sandbox_argv_for_plan(&docker_plan, "sleep 60")
            .unwrap()
            .unwrap();
        assert!(run_argv
            .windows(2)
            .any(|pair| pair[0] == "--name" && pair[1] == name));
        let cleanup_argv = sandbox_cleanup_argv_for_plan(&docker_plan).unwrap();
        assert_eq!(cleanup_argv, vec!["docker", "rm", "-f", &name]);

        let mut msb_plan = plan(ShellExecutionBackendKind::Msb);
        msb_plan.tool_call_id = "Tool Call/ABC 123".to_string();
        let msb_argv = sandbox_argv_for_plan(&msb_plan, "sleep 60")
            .unwrap()
            .unwrap();
        assert!(msb_argv
            .windows(2)
            .any(|pair| pair[0] == "--name" && pair[1] == name));
        assert_eq!(
            sandbox_cleanup_argv_for_plan(&msb_plan).unwrap(),
            vec!["msb", "remove", "--force", "--quiet", &name]
        );
    }

    #[test]
    fn host_plan_returns_no_sandbox_command() {
        let plan = plan(ShellExecutionBackendKind::Host);
        assert!(sandbox_command_for_plan(&plan, "echo hi")
            .unwrap()
            .is_none());
    }
}
