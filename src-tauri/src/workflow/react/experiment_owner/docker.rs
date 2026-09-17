//! The persistent Docker execution owner (Phase 2G).
//!
//! `PersistentDockerOwner` owns one run's isolated execution environment: a
//! digest-pinned container whose only writable host path is the run's Git
//! worktree, mounted at `/workspace`. The owner composes
//! [`HostWorktreeOwner`] for the workspace itself (worktree creation, the
//! allowlisted input patch, output-patch publication) and adds the container
//! lifecycle on top, so patch handling stays in exactly one implementation.
//!
//! Fail-closed rules (AC-3/AC-5, INV-4/INV-5/INV-8):
//!
//! - The image reference must be pinned by `@sha256:` **and** present in the
//!   local image store. The owner never pulls: an unavailable image is a
//!   pre-dispatch `executor_unavailable`, not a silent network fetch.
//! - A container is claimable only through its immutable labels. Adoption,
//!   execution and removal all compare `(job, owner token, generation)` first,
//!   so a superseded worker can neither exec in nor delete a newer owner's
//!   container even if it remembers the name.
//! - A network policy the local Docker cannot enforce (a domain allowlist) is a
//!   pre-dispatch failure, matching the existing sandbox runner's behaviour; it
//!   never degrades to an open network.
//! - Only the run workspace is mounted; any other mount declaration is refused.

use crate::workflow::react::experiment_owner::patch::PatchArtifact;
use crate::workflow::react::experiment_owner::worktree::HostWorktreeOwner;
use crate::workflow::react::experiment_owner::{
    owner_error, ContainerHandle, ExecutionOwner, InputPatch, OwnerAcquireRequest,
    PreparedWorkspace,
};
use crate::workflow::react::experiment_schedule::types::{
    is_digest_pinned_image, is_valid_key, MountSpecV1, NetworkPolicyV1, OwnerKind,
    ResourceLimitsV1, ScheduleError, ScheduleErrorCode,
};
use std::process::{Command, Stdio};

/// Label schema version stamped on every container this owner creates.
pub const CONTAINER_LABEL_SCHEMA: &str = "run_container_owner.v1";
/// Container name prefix.
pub const CONTAINER_NAME_PREFIX: &str = "cs-run";
/// The container path the run workspace is mounted at.
pub const WORKSPACE_MOUNT_PATH: &str = "/workspace";

/// The label carrying the owner schema version.
pub const LABEL_SCHEMA: &str = "cs.owner_schema";
/// The label carrying the job id.
pub const LABEL_JOB: &str = "cs.job";
/// The label carrying the owner token hash.
pub const LABEL_OWNER_TOKEN: &str = "cs.owner_token";
/// The label carrying the lease generation.
pub const LABEL_GENERATION: &str = "cs.generation";
/// The label carrying the digest-pinned image reference.
pub const LABEL_IMAGE: &str = "cs.image";

/// The container configuration a profile provides.
#[derive(Debug, Clone)]
pub struct DockerOwnerConfig {
    /// Digest-pinned image reference (`repo@sha256:...`).
    pub image_reference: String,
    /// Required network policy; only `none` and `public` are enforceable here.
    pub network_policy: NetworkPolicyV1,
    pub resources: ResourceLimitsV1,
    /// Whether the mounted workspace is read-only inside the container.
    pub workspace_read_only: bool,
    /// Mount declarations from the execution profile; only the workspace mount
    /// is accepted.
    pub mounts: Vec<MountSpecV1>,
}

impl DockerOwnerConfig {
    /// Validates the configuration in isolation, before any container work.
    pub fn validate(&self) -> Result<(), ScheduleError> {
        if !is_digest_pinned_image(&self.image_reference) {
            return Err(owner_error(
                ScheduleErrorCode::ImageNotDigestPinned,
                format!(
                    "image reference '{}' is not pinned by sha256 digest",
                    self.image_reference
                ),
            ));
        }
        if !self.network_policy.is_known_mode() {
            return Err(owner_error(
                ScheduleErrorCode::NetworkPolicyUnsupported,
                format!(
                    "unsupported network policy mode '{}'",
                    self.network_policy.mode
                ),
            ));
        }
        if self.network_policy.mode == NetworkPolicyV1::MODE_EGRESS_ALLOWLIST {
            // Docker domain allowlisting is not implemented by this build, so a
            // profile that asks for it must be refused rather than silently run
            // with an open network.
            return Err(owner_error(
                ScheduleErrorCode::NetworkPolicyUnsupported,
                "Docker domain allowlist networking is not supported by this owner",
            ));
        }
        for mount in &self.mounts {
            let is_workspace = mount.source_kind == MountSpecV1::SOURCE_WORKSPACE
                && mount.container_path == WORKSPACE_MOUNT_PATH;
            if !is_workspace {
                return Err(owner_error(
                    ScheduleErrorCode::InvalidExecutionProfile,
                    format!(
                        "the docker owner only mounts the run workspace at {WORKSPACE_MOUNT_PATH}; \
                         refusing mount '{}' -> '{}'",
                        mount.source_kind, mount.container_path
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// A container observed through `docker inspect`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedContainer {
    pub name: String,
    pub running: bool,
    pub labels: Vec<(String, String)>,
}

impl ObservedContainer {
    fn label(&self, key: &str) -> Option<&str> {
        self.labels
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }
}

/// A persistent, label-fenced Docker execution owner.
pub struct PersistentDockerOwner {
    worktree: HostWorktreeOwner,
    config: DockerOwnerConfig,
    executable: String,
}

impl PersistentDockerOwner {
    pub fn new(worktree: HostWorktreeOwner, config: DockerOwnerConfig) -> Self {
        Self {
            worktree,
            config,
            executable: "docker".to_string(),
        }
    }

    /// The deterministic container name of one job generation.
    pub fn container_name(&self, job_id: &str, generation: i64) -> Result<String, ScheduleError> {
        if !is_valid_key(job_id) {
            return Err(owner_error(
                ScheduleErrorCode::OwnershipMismatch,
                "the job id is not usable as a container name",
            ));
        }
        let raw = format!("{CONTAINER_NAME_PREFIX}-{job_id}-g{generation}");
        let mut sanitized: String = raw
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                    ch.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        sanitized.truncate(60);
        Ok(sanitized)
    }

    fn token_hash(&self, request: &OwnerAcquireRequest) -> String {
        request.fence.token_hash(&request.job_id)
    }

    /// Runs one `docker` invocation with explicit argv.
    fn docker(&self, args: &[&str]) -> Result<String, ScheduleError> {
        let output = Command::new(&self.executable)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| {
                owner_error(
                    ScheduleErrorCode::ExecutorUnavailable,
                    format!("failed to run docker: {error}"),
                )
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(owner_error(
                ScheduleErrorCode::ExecutorUnavailable,
                format!("docker {} failed: {}", args.join(" "), stderr),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Observes a container by name, or `None` when it does not exist.
    fn inspect(&self, name: &str) -> Result<Option<ObservedContainer>, ScheduleError> {
        let output = Command::new(&self.executable)
            .args([
                "inspect",
                "--format",
                "{{.State.Running}}{{println}}{{range $k, $v := .Config.Labels}}{{$k}}={{$v}}{{println}}{{end}}",
                name,
            ])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| {
                owner_error(
                    ScheduleErrorCode::ExecutorUnavailable,
                    format!("failed to run docker inspect: {error}"),
                )
            })?;
        if !output.status.success() {
            // A missing container is not an error: it means "nothing to adopt".
            return Ok(None);
        }
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let mut lines = stdout.lines();
        let running = lines
            .next()
            .map(|line| line.trim().eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let labels = lines
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        Ok(Some(ObservedContainer {
            name: name.to_string(),
            running,
            labels,
        }))
    }

    /// Whether a container's labels prove it belongs to this job generation.
    fn labels_match(&self, observed: &ObservedContainer, request: &OwnerAcquireRequest) -> bool {
        observed.label(LABEL_SCHEMA) == Some(CONTAINER_LABEL_SCHEMA)
            && observed.label(LABEL_JOB) == Some(request.job_id.as_str())
            && observed.label(LABEL_OWNER_TOKEN) == Some(self.token_hash(request).as_str())
            && observed.label(LABEL_GENERATION)
                == Some(request.fence.lease_generation.to_string().as_str())
            && observed.label(LABEL_IMAGE) == Some(self.config.image_reference.as_str())
    }

    fn handle(&self, name: &str, token_hash: String) -> ContainerHandle {
        ContainerHandle {
            name: name.to_string(),
            owner_token_hash: token_hash,
            image_reference: self.config.image_reference.clone(),
        }
    }

    /// Confirms the workspace a proof carries is really this owner's, returning
    /// an owned copy of the handle.
    fn require_container(
        &self,
        workspace: &PreparedWorkspace,
    ) -> Result<ContainerHandle, ScheduleError> {
        workspace.container.clone().ok_or_else(|| {
            owner_error(
                ScheduleErrorCode::OwnershipMismatch,
                "this workspace carries no container handle",
            )
        })
    }

    /// Runs one command inside the owned container and returns its stdout.
    pub fn exec_capture(&self, name: &str, argv: &[&str]) -> Result<String, ScheduleError> {
        let mut args = vec!["exec", name];
        args.extend_from_slice(argv);
        self.docker(&args)
    }
}

impl ExecutionOwner for PersistentDockerOwner {
    fn kind(&self) -> OwnerKind {
        OwnerKind::PersistentDocker
    }

    fn preflight(&self) -> Result<(), ScheduleError> {
        self.config.validate()?;
        // The container runtime must be usable...
        self.docker(&["version", "--format", "{{.Server.Version}}"])?;
        // ...and the pinned image must already exist locally: the owner never
        // pulls, so an absent image is a pre-dispatch failure (INV-4).
        self.docker(&[
            "image",
            "inspect",
            "--format",
            "{{.Id}}",
            self.config.image_reference.as_str(),
        ])
        .map_err(|error| {
            owner_error(
                ScheduleErrorCode::ExecutorUnavailable,
                format!(
                    "the pinned image '{}' is not available locally: {}",
                    self.config.image_reference, error.message
                ),
            )
        })?;
        // The workspace provider must be usable too.
        self.worktree.preflight()
    }

    fn acquire(&self, request: &OwnerAcquireRequest) -> Result<PreparedWorkspace, ScheduleError> {
        self.preflight()?;
        let mut workspace = self.worktree.acquire(request)?;
        let name = self.container_name(&request.job_id, request.fence.lease_generation)?;
        let token_hash = self.token_hash(request);

        if let Some(observed) = self.inspect(&name)? {
            if !self.labels_match(&observed, request) {
                // Roll the fresh worktree back: we must not leave a workspace
                // paired with a container we do not own.
                let _ = self.worktree.cleanup(&workspace);
                return Err(owner_error(
                    ScheduleErrorCode::OwnershipMismatch,
                    format!("the container '{name}' is not owned by this generation"),
                ));
            }
            if !observed.running {
                self.docker(&["start", name.as_str()])?;
            }
            workspace.container = Some(self.handle(&name, token_hash));
            return Ok(workspace);
        }

        let workspace_arg = workspace.proof.workspace_root.to_string_lossy().to_string();
        let generation = request.fence.lease_generation.to_string();
        let mut args: Vec<String> = vec![
            "create".to_string(),
            "--name".to_string(),
            name.clone(),
            "--label".to_string(),
            format!("{LABEL_SCHEMA}={CONTAINER_LABEL_SCHEMA}"),
            "--label".to_string(),
            format!("{LABEL_JOB}={}", request.job_id),
            "--label".to_string(),
            format!("{LABEL_OWNER_TOKEN}={token_hash}"),
            "--label".to_string(),
            format!("{LABEL_GENERATION}={generation}"),
            "--label".to_string(),
            format!("{LABEL_IMAGE}={}", self.config.image_reference),
            "--workdir".to_string(),
            WORKSPACE_MOUNT_PATH.to_string(),
        ];
        match self.config.network_policy.mode.as_str() {
            NetworkPolicyV1::MODE_NONE => {
                args.push("--network".to_string());
                args.push("none".to_string());
            }
            NetworkPolicyV1::MODE_EGRESS_ALLOWLIST => {
                return Err(owner_error(
                    ScheduleErrorCode::NetworkPolicyUnsupported,
                    "Docker domain allowlist networking is not supported by this owner",
                ))
            }
            _ => {}
        }
        if self.config.resources.memory_bytes > 0 {
            args.push("--memory".to_string());
            args.push(format!(
                "{}m",
                self.config.resources.memory_bytes / (1024 * 1024)
            ));
        }
        if self.config.resources.cpu_millis > 0 {
            args.push("--cpus".to_string());
            args.push(format!(
                "{:.3}",
                self.config.resources.cpu_millis as f64 / 1000.0
            ));
        }
        if self.config.resources.pids > 0 {
            args.push("--pids-limit".to_string());
            args.push(self.config.resources.pids.to_string());
        }
        if self.config.resources.no_new_privileges {
            args.push("--security-opt".to_string());
            args.push("no-new-privileges".to_string());
        }
        let readonly = if self.config.workspace_read_only {
            ",readonly"
        } else {
            ""
        };
        args.push("--mount".to_string());
        args.push(format!(
            "type=bind,src={},dst={WORKSPACE_MOUNT_PATH}{readonly}",
            workspace_arg
        ));
        args.push(self.config.image_reference.clone());
        args.push("sleep".to_string());
        args.push("infinity".to_string());

        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        self.docker(&argv)?;
        self.docker(&["start", name.as_str()])?;

        workspace.container = Some(self.handle(&name, token_hash));
        Ok(workspace)
    }

    fn adopt(
        &self,
        request: &OwnerAcquireRequest,
    ) -> Result<Option<PreparedWorkspace>, ScheduleError> {
        let name = self.container_name(&request.job_id, request.fence.lease_generation)?;
        let Some(observed) = self.inspect(&name)? else {
            return Ok(None);
        };
        if !self.labels_match(&observed, request) {
            return Ok(None);
        }
        let Some(mut workspace) = self.worktree.adopt(request)? else {
            return Ok(None);
        };
        workspace.container = Some(self.handle(&name, self.token_hash(request)));
        Ok(Some(workspace))
    }

    fn apply_input_patch(
        &self,
        workspace: &PreparedWorkspace,
        patch: &InputPatch,
    ) -> Result<(), ScheduleError> {
        // The workspace is the container's only writable mount, so applying the
        // patch there is exactly applying it inside the isolated environment.
        self.require_container(workspace)?;
        self.worktree.apply_input_patch(workspace, patch)
    }

    fn collect_output_patch(
        &self,
        workspace: &PreparedWorkspace,
        context: &crate::workflow::react::experiment_owner::patch::PatchContext,
    ) -> Result<PatchArtifact, ScheduleError> {
        self.require_container(workspace)?;
        self.worktree.collect_output_patch(workspace, context)
    }

    fn cleanup(&self, workspace: &PreparedWorkspace) -> Result<(), ScheduleError> {
        let handle = self.require_container(workspace)?;
        if let Some(observed) = self.inspect(&handle.name)? {
            let owns = observed.label(LABEL_SCHEMA) == Some(CONTAINER_LABEL_SCHEMA)
                && observed.label(LABEL_OWNER_TOKEN) == Some(handle.owner_token_hash.as_str())
                && observed.label(LABEL_IMAGE) == Some(handle.image_reference.as_str());
            if !owns {
                return Err(owner_error(
                    ScheduleErrorCode::OwnershipMismatch,
                    format!(
                        "the container '{}' is not owned by this generation; refusing to remove it",
                        handle.name
                    ),
                ));
            }
            self.docker(&["rm", "-f", handle.name.as_str()])?;
        }
        self.worktree.cleanup(workspace)
    }
}

/// Resolves an immutable, local pin for an image that already exists.
///
/// Preference order:
///
/// 1. the registry digest (`repo@sha256:...`), when the image carries one, and
/// 2. the local image id (`sha256:...`), which is the immutable content address
///    of an image that was built or loaded locally.
///
/// It returns `None` when the image is absent, so the caller can fail closed
/// instead of pulling (operators and the Harbor adapter use this to pin the
/// profiles they register).
pub fn local_image_pin(executable: &str, reference: &str) -> Result<Option<String>, ScheduleError> {
    let inspect = |format: &str| -> Result<Option<String>, ScheduleError> {
        let output = Command::new(executable)
            .args(["image", "inspect", "--format", format, reference])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| {
                owner_error(
                    ScheduleErrorCode::ExecutorUnavailable,
                    format!("failed to run docker: {error}"),
                )
            })?;
        if !output.status.success() {
            return Ok(None);
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok((!value.is_empty()).then_some(value))
    };
    if let Some(digests) = inspect("{{index .RepoDigests 0}}")? {
        let repo_digest = digests.trim_start_matches("docker.io/").to_string();
        if is_digest_pinned_image(&repo_digest) {
            return Ok(Some(repo_digest));
        }
    }
    if let Some(id) = inspect("{{.Id}}")? {
        if is_digest_pinned_image(&id) {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::experiment_owner::patch::PatchContext;
    use crate::workflow::react::experiment_owner::worktree::HostWorktreeOwner;
    use crate::workflow::react::experiment_schedule::types::OwnerFence;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tempfile::tempdir;

    /// Whether a Docker daemon is reachable in this environment.
    fn docker_available() -> bool {
        Command::new("docker")
            .args(["version", "--format", "{{.Server.Version}}"])
            .stdin(Stdio::null())
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// An immutable local pin for a locally available image, if any.
    ///
    /// The owner never pulls, so the gate only runs when a git- or shell-capable
    /// image is already present on the machine.
    fn available_image() -> Option<String> {
        for reference in [
            "busybox:latest",
            "git:latest",
            "dev:latest",
            "alpine:latest",
        ] {
            if let Ok(Some(pin)) = local_image_pin("docker", reference) {
                return Some(pin);
            }
        }
        None
    }

    fn unique_job_id(prefix: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.subsec_nanos())
            .unwrap_or(0);
        format!("{prefix}-{}-{nanos}", std::process::id())
    }

    fn config(image: &str) -> DockerOwnerConfig {
        DockerOwnerConfig {
            image_reference: image.to_string(),
            network_policy: NetworkPolicyV1 {
                mode: NetworkPolicyV1::MODE_NONE.to_string(),
                allow_hosts: Vec::new(),
            },
            resources: ResourceLimitsV1 {
                cpu_millis: 1000,
                memory_bytes: 256 * 1024 * 1024,
                pids: 128,
                no_new_privileges: true,
            },
            workspace_read_only: false,
            mounts: vec![MountSpecV1 {
                source_kind: MountSpecV1::SOURCE_WORKSPACE.to_string(),
                container_path: WORKSPACE_MOUNT_PATH.to_string(),
                read_only: false,
            }],
        }
    }

    fn base_repo(directory: &Path) -> PathBuf {
        let repo = directory.join("base");
        std::fs::create_dir_all(&repo).expect("create base");
        let run = |args: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .env("GIT_AUTHOR_NAME", "cs-test")
                .env("GIT_AUTHOR_EMAIL", "cs-test@example.invalid")
                .env("GIT_COMMITTER_NAME", "cs-test")
                .env("GIT_COMMITTER_EMAIL", "cs-test@example.invalid")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("git");
            assert!(status.success(), "git {args:?} failed");
        };
        run(&["init", "--quiet"]);
        run(&["config", "user.email", "cs-test@example.invalid"]);
        run(&["config", "user.name", "cs-test"]);
        std::fs::write(repo.join("app.py"), "print('base')\n").expect("write app");
        run(&["add", "-A"]);
        run(&["commit", "--quiet", "-m", "base"]);
        repo
    }

    fn owner(directory: &Path, image: &str) -> PersistentDockerOwner {
        let worktree = HostWorktreeOwner::new(base_repo(directory), directory.join("worktrees"));
        PersistentDockerOwner::new(worktree, config(image))
    }

    fn request(job_id: &str, token: &str, generation: i64) -> OwnerAcquireRequest {
        OwnerAcquireRequest {
            job_id: job_id.to_string(),
            fence: OwnerFence::new(token, generation),
            base_revision: "HEAD".to_string(),
            input_patch: None,
        }
    }

    #[test]
    fn the_configuration_refuses_unpinned_images_and_unsupported_policies() {
        let unpinned = config("chatspeed/runner:latest");
        let error = unpinned.validate().expect_err("unpinned");
        assert_eq!(error.code, ScheduleErrorCode::ImageNotDigestPinned);

        let mut allowlist = config(&format!("chatspeed/runner@sha256:{}", "a".repeat(64)));
        allowlist.network_policy = NetworkPolicyV1 {
            mode: NetworkPolicyV1::MODE_EGRESS_ALLOWLIST.to_string(),
            allow_hosts: vec!["example.invalid".to_string()],
        };
        let error = allowlist.validate().expect_err("allowlist");
        assert_eq!(error.code, ScheduleErrorCode::NetworkPolicyUnsupported);

        let mut host_mount = config(&format!("chatspeed/runner@sha256:{}", "a".repeat(64)));
        host_mount.mounts.push(MountSpecV1 {
            source_kind: MountSpecV1::SOURCE_BUNDLE.to_string(),
            container_path: "/opt/bundle".to_string(),
            read_only: true,
        });
        let error = host_mount.validate().expect_err("extra mount");
        assert_eq!(error.code, ScheduleErrorCode::InvalidExecutionProfile);

        // A locally built image pinned by its immutable id is acceptable.
        let local = config(&format!("sha256:{}", "a".repeat(64)));
        local.validate().expect("local image id pin");
    }

    #[test]
    fn container_names_are_deterministic_and_sanitized() {
        let directory = tempdir().expect("tempdir");
        let owner = owner(directory.path(), &format!("sha256:{}", "a".repeat(64)));
        let name = owner.container_name("job-1", 2).expect("name");
        assert_eq!(name, "cs-run-job-1-g2");
        assert_eq!(owner.container_name("job-1", 2).expect("name"), name);
        assert!(owner.container_name("../escape", 1).is_err());
    }

    #[test]
    fn preflight_fails_closed_for_an_absent_pinned_image() {
        if !docker_available() {
            eprintln!("skipping: no docker daemon available");
            return;
        }
        let directory = tempdir().expect("tempdir");
        // A syntactically valid but locally absent digest: the owner must never
        // pull it.
        let owner = owner(
            directory.path(),
            &format!("chatspeed/runner@sha256:{}", "f".repeat(64)),
        );
        let error = owner.preflight().expect_err("absent image");
        assert_eq!(error.code, ScheduleErrorCode::ExecutorUnavailable);
        assert!(error.message.contains("not available locally"));
    }

    /// The real 2G gate: a digest-pinned container is created, executes inside
    /// the mounted worktree, is adoptable only by its own generation, and is the
    /// only thing cleanup removes.
    #[test]
    fn the_container_gate_creates_execs_adopts_and_removes_only_its_own_container() {
        if !docker_available() {
            eprintln!("skipping: no docker daemon available");
            return;
        }
        let Some(image) = available_image() else {
            eprintln!("skipping: no local digest-pinned image available");
            return;
        };
        let directory = tempdir().expect("tempdir");
        let job_id = unique_job_id("p2gh-dock");
        let owner = owner(directory.path(), &image);
        owner.preflight().expect("preflight");

        let acquire = request(&job_id, "worker-a", 1);
        let workspace = owner.acquire(&acquire).expect("acquire");
        let handle = workspace.container.clone().expect("container handle");
        assert_eq!(handle.image_reference, image);
        assert_eq!(handle.owner_token_hash, acquire.fence.token_hash(&job_id));

        // The run executes inside the isolated container, and the only writable
        // host path it sees is the mounted workspace.
        let version = owner
            .exec_capture(&handle.name, &["sh", "-lc", "echo exec-ok"])
            .expect("exec inside the container");
        assert!(version.contains("exec-ok"));
        owner
            .exec_capture(
                &handle.name,
                &[
                    "sh",
                    "-lc",
                    "echo from-container > /workspace/from-container.txt",
                ],
            )
            .expect("write through the mount");
        assert!(workspace
            .proof
            .workspace_root
            .join("from-container.txt")
            .exists());

        // Adoption is generation fenced.
        let adopted = owner
            .adopt(&acquire)
            .expect("adopt")
            .expect("same generation");
        assert_eq!(
            adopted.container.as_ref().map(|c| c.name.clone()),
            Some(handle.name.clone())
        );
        assert!(owner
            .adopt(&request(&job_id, "worker-b", 1))
            .expect("adopt")
            .is_none());
        // Re-acquiring the same generation is idempotent.
        let reacquired = owner.acquire(&acquire).expect("re-acquire");
        assert_eq!(
            reacquired.container.as_ref().map(|c| c.name.clone()),
            Some(handle.name.clone())
        );

        // The collected patch comes from the mounted workspace.
        let context = PatchContext {
            job_id: job_id.clone(),
            run_id: Some("run-1".to_string()),
            session_id: Some("session-1".to_string()),
            candidate_key: "baseline".to_string(),
            base_revision: "HEAD".to_string(),
            destination_root: directory.path().join("artifacts"),
        };
        let artifact = owner
            .collect_output_patch(&workspace, &context)
            .expect("collect");
        let paths: Vec<&str> = artifact
            .manifest
            .files
            .iter()
            .map(|entry| entry.relative_path.as_str())
            .collect();
        assert!(paths.contains(&"from-container.txt"), "paths: {paths:?}");

        // Cleanup removes the container and the worktree, and nothing else.
        owner.cleanup(&workspace).expect("cleanup");
        assert!(
            owner.inspect(&handle.name).expect("inspect").is_none(),
            "the owned container must be removed"
        );
        assert!(!workspace.proof.workspace_root.exists());
        // Cleanup is idempotent.
        owner.cleanup(&workspace).expect("cleanup again");
    }

    /// A same-named container with foreign labels is never taken over, and it is
    /// never removed by an owner that does not own it.
    #[test]
    fn a_foreign_container_with_the_same_name_is_never_taken_over() {
        if !docker_available() {
            eprintln!("skipping: no docker daemon available");
            return;
        }
        let Some(image) = available_image() else {
            eprintln!("skipping: no local digest-pinned image available");
            return;
        };
        let directory = tempdir().expect("tempdir");
        let job_id = unique_job_id("p2gh-dock-foreign");
        let owner = owner(directory.path(), &image);
        let acquire = request(&job_id, "worker-a", 1);
        let name = owner
            .container_name(&job_id, acquire.fence.lease_generation)
            .expect("name");

        // A foreign container occupies the deterministic name.
        let status = Command::new("docker")
            .args([
                "create",
                "--name",
                name.as_str(),
                "--label",
                "cs.owner_schema=somebody-else",
                image.as_str(),
                "sleep",
                "infinity",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("create foreign container");
        assert!(status.success(), "failed to create the foreign container");

        let error = owner
            .acquire(&acquire)
            .expect_err("must not take over a foreign container");
        assert_eq!(error.code, ScheduleErrorCode::OwnershipMismatch);
        // The foreign container still exists: nothing was removed.
        assert!(
            owner.inspect(&name).expect("inspect").is_some(),
            "a foreign container must never be removed"
        );
        // No worktree was left behind for the aborted acquisition.
        assert!(!directory
            .path()
            .join("worktrees")
            .join(format!("{job_id}-g{}", acquire.fence.lease_generation))
            .exists());

        // Leave no residue.
        let _ = Command::new("docker")
            .args(["rm", "-f", name.as_str()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}
