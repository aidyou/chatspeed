//! The Harbor task-environment execution owner (Phase 2G/2H).
//!
//! A Harbor trial runs the agent *inside a task environment that Harbor itself
//! provisioned*. There is no container for ChatSpeed to create there, so this
//! owner's job is not orchestration but **proof**: it accepts an environment as
//! its own only when the Harbor adapter wrote a current-user-only capability
//! manifest, and it confines every path it uses to the roots that manifest
//! declares.
//!
//! That makes the Harbor gate a real isolation gate rather than a fallback:
//!
//! - without a valid capability manifest there is no owner, and the scheduler
//!   fails closed (`unsupported_owner_kind` / `ownership_mismatch`) instead of
//!   silently running on the host (INV-4);
//! - an input patch and the collected output patch are confined to the declared
//!   workspace and artifact roots, so a task can never write outside its own
//!   sandbox;
//! - cleanup never deletes the task environment: Harbor owns it, and destroying
//!   it would destroy the trial's own evidence.

use crate::workflow::react::experiment_owner::patch::{self, PatchArtifact, PatchContext};
use crate::workflow::react::experiment_owner::{
    owner_error, ExecutionOwner, InputPatch, OwnerAcquireRequest, PreparedWorkspace, WorkspaceProof,
};
use crate::workflow::react::experiment_schedule::types::{
    HarborTaskCapabilityV1, OwnerKind, ScheduleError, ScheduleErrorCode,
};
use std::path::{Path, PathBuf};

/// Fixed file name of the adapter-written capability manifest.
pub const HARBOR_CAPABILITY_FILE_NAME: &str = "harbor-task-capability.json";

/// An execution owner for the current Harbor task environment.
#[derive(Debug, Clone)]
pub struct HarborTaskOwner {
    capability: HarborTaskCapabilityV1,
    capability_path: PathBuf,
}

impl HarborTaskOwner {
    /// Loads and validates the capability manifest written by the adapter.
    pub fn load(capability_path: impl AsRef<Path>) -> Result<Self, ScheduleError> {
        let capability_path = capability_path.as_ref().to_path_buf();
        let body = std::fs::read(&capability_path).map_err(|error| {
            owner_error(
                ScheduleErrorCode::OwnershipMismatch,
                format!(
                    "no harbor task capability manifest at '{}': {error}",
                    capability_path.display()
                ),
            )
        })?;
        // The manifest carries the owner token hash, so it must not be readable
        // by anyone but the current user.
        require_current_user_only(&capability_path)?;
        let capability: HarborTaskCapabilityV1 =
            serde_json::from_slice(&body).map_err(|error| {
                owner_error(
                    ScheduleErrorCode::UnsupportedOwnerKind,
                    format!(
                        "'{}' is not a valid harbor task capability manifest: {error}",
                        capability_path.display()
                    ),
                )
            })?;
        capability.validate()?;
        Ok(Self {
            capability,
            capability_path,
        })
    }

    /// Convenience constructor for tests and adapters that already hold a
    /// validated capability in memory.
    pub fn from_capability(
        capability: HarborTaskCapabilityV1,
        capability_path: impl Into<PathBuf>,
    ) -> Result<Self, ScheduleError> {
        capability.validate()?;
        Ok(Self {
            capability,
            capability_path: capability_path.into(),
        })
    }

    pub fn capability_path(&self) -> &Path {
        &self.capability_path
    }

    /// Confirms a path the owner is about to use lives inside a declared root.
    fn require_owned(&self, path: &Path, what: &str) -> Result<(), ScheduleError> {
        let rendered = path.to_string_lossy().to_string();
        if !self.capability.contains_owned_path(&rendered) {
            return Err(owner_error(
                ScheduleErrorCode::WorkspaceEscape,
                format!(
                    "{what} '{}' is outside every root declared by the harbor capability",
                    path.display()
                ),
            ));
        }
        Ok(())
    }

    /// Confirms a proof really belongs to this capability.
    fn require_proof(&self, workspace: &PreparedWorkspace) -> Result<(), ScheduleError> {
        if workspace.proof.owner_token_hash != self.capability.owner_token_hash {
            return Err(owner_error(
                ScheduleErrorCode::OwnershipMismatch,
                "the workspace proof does not belong to the current harbor capability",
            ));
        }
        self.require_owned(&workspace.proof.workspace_root, "the workspace root")
    }

    /// The artifact root the owner publishes into (the first declared one).
    pub fn artifact_root(&self) -> Result<PathBuf, ScheduleError> {
        self.capability
            .artifact_roots
            .first()
            .map(PathBuf::from)
            .ok_or_else(|| {
                owner_error(
                    ScheduleErrorCode::OwnershipMismatch,
                    "the harbor capability declares no artifact root",
                )
            })
    }
}

impl ExecutionOwner for HarborTaskOwner {
    fn kind(&self) -> OwnerKind {
        OwnerKind::HarborTask
    }

    /// The declared artifact root: Harbor collects exactly this tree, so the
    /// run's patch evidence is published inside it rather than beside the
    /// domain (which the capability would refuse as a workspace escape).
    fn artifact_root(&self) -> Option<PathBuf> {
        self.artifact_root().ok()
    }

    fn preflight(&self) -> Result<(), ScheduleError> {
        self.capability.validate()?;
        for (path, what) in [
            (&self.capability.task_root, "the task root"),
            (&self.capability.workspace_root, "the workspace root"),
        ] {
            let path = Path::new(path);
            if !path.is_dir() {
                return Err(owner_error(
                    ScheduleErrorCode::OwnershipMismatch,
                    format!(
                        "{what} '{}' does not exist in this environment",
                        path.display()
                    ),
                ));
            }
        }
        for root in self
            .capability
            .artifact_roots
            .iter()
            .chain(self.capability.read_only_roots.iter())
        {
            let path = Path::new(root);
            if !path.exists() {
                return Err(owner_error(
                    ScheduleErrorCode::OwnershipMismatch,
                    format!(
                        "the declared root '{}' does not exist in this environment",
                        path.display()
                    ),
                ));
            }
        }
        // The workspace must live inside the task root, so the run cannot write
        // anywhere else in the environment by relative traversal.
        if !self
            .capability
            .contains_owned_path(&self.capability.workspace_root)
        {
            return Err(owner_error(
                ScheduleErrorCode::WorkspaceEscape,
                "the workspace root is not inside a declared root",
            ));
        }
        Ok(())
    }

    fn acquire(&self, request: &OwnerAcquireRequest) -> Result<PreparedWorkspace, ScheduleError> {
        self.preflight()?;
        // Harbor owns the environment, so acquiring means *binding* to it, not
        // creating anything.
        Ok(PreparedWorkspace {
            proof: WorkspaceProof {
                job_id: request.job_id.clone(),
                owner_token_hash: self.capability.owner_token_hash.clone(),
                base_revision: request.base_revision.clone(),
                workspace_root: PathBuf::from(&self.capability.workspace_root),
                worktrees_root: PathBuf::from(&self.capability.task_root),
            },
            input_patch_applied: false,
            container: None,
        })
    }

    fn adopt(
        &self,
        request: &OwnerAcquireRequest,
    ) -> Result<Option<PreparedWorkspace>, ScheduleError> {
        // A restart inside the same task environment adopts the same sandbox.
        self.acquire(request).map(Some)
    }

    fn apply_input_patch(
        &self,
        workspace: &PreparedWorkspace,
        patch: &InputPatch,
    ) -> Result<(), ScheduleError> {
        self.require_proof(workspace)?;
        patch::apply_patch_in_workspace(&workspace.proof.workspace_root, patch)
    }

    fn collect_output_patch(
        &self,
        workspace: &PreparedWorkspace,
        context: &PatchContext,
    ) -> Result<PatchArtifact, ScheduleError> {
        self.require_proof(workspace)?;
        // Publication must land inside a declared artifact root: the verifier
        // only ever receives declared artifacts, so writing elsewhere would
        // silently lose the evidence.
        self.require_owned(&context.destination_root, "the artifact destination")?;
        patch::collect_workspace_patch(
            &workspace.proof.workspace_root,
            context,
            crate::headless::domain::now_ms(),
        )
    }

    fn cleanup(&self, workspace: &PreparedWorkspace) -> Result<(), ScheduleError> {
        // Ownership is still verified, but the environment itself is never
        // removed: Harbor owns the sandbox and destroying it would destroy the
        // trial's evidence. Only the caller's own bookkeeping is dropped.
        self.require_proof(workspace)?;
        Ok(())
    }
}

/// Requires the capability manifest to be readable only by its owner.
#[cfg(unix)]
fn require_current_user_only(path: &Path) -> Result<(), ScheduleError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)
        .map_err(|error| {
            owner_error(
                ScheduleErrorCode::OwnershipMismatch,
                format!("failed to inspect '{}': {error}", path.display()),
            )
        })?
        .permissions()
        .mode()
        & 0o777;
    if mode & 0o077 != 0 {
        return Err(owner_error(
            ScheduleErrorCode::OwnershipMismatch,
            format!(
                "the harbor capability manifest '{}' is readable by other users (mode {mode:o})",
                path.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn require_current_user_only(_path: &Path) -> Result<(), ScheduleError> {
    // On Windows the manifest inherits the current-user ACL of its directory.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::experiment_owner::patch::PatchContext;
    use crate::workflow::react::experiment_schedule::types::{
        NetworkPolicyV1, HARBOR_TASK_CAPABILITY_V1,
    };
    use std::process::{Command, Stdio};
    use tempfile::tempdir;

    /// Builds a capability over a real task-shaped directory layout.
    fn capability(root: &Path, token: &str) -> HarborTaskCapabilityV1 {
        let task = root.join("task");
        let workspace = task.join("workspace");
        let artifacts = root.join("artifacts");
        let fixtures = task.join("fixtures");
        for directory in [&workspace, &artifacts, &fixtures] {
            std::fs::create_dir_all(directory).expect("create root");
        }
        HarborTaskCapabilityV1 {
            schema_version: HARBOR_TASK_CAPABILITY_V1.to_string(),
            task_id: "trial-1".to_string(),
            nonce: "nonce-1".to_string(),
            owner_token_hash: token.to_string(),
            task_root: task.to_string_lossy().to_string(),
            workspace_root: workspace.to_string_lossy().to_string(),
            artifact_roots: vec![artifacts.to_string_lossy().to_string()],
            network_policy: NetworkPolicyV1 {
                mode: NetworkPolicyV1::MODE_NONE.to_string(),
                allow_hosts: Vec::new(),
            },
            read_only_roots: vec![fixtures.to_string_lossy().to_string()],
            issued_at: "2026-09-16T00:00:00Z".to_string(),
        }
    }

    fn git_in(directory: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(args)
            .env("GIT_AUTHOR_NAME", "cs-test")
            .env("GIT_AUTHOR_EMAIL", "cs-test@example.invalid")
            .env("GIT_COMMITTER_NAME", "cs-test")
            .env("GIT_COMMITTER_EMAIL", "cs-test@example.invalid")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn init_workspace_repo(workspace: &Path) {
        git_in(workspace, &["init", "--quiet"]);
        git_in(
            workspace,
            &["config", "user.email", "cs-test@example.invalid"],
        );
        git_in(workspace, &["config", "user.name", "cs-test"]);
        std::fs::write(workspace.join("app.py"), "print('base')\n").expect("write app");
        git_in(workspace, &["add", "-A"]);
        git_in(workspace, &["commit", "--quiet", "-m", "base"]);
    }

    fn request() -> OwnerAcquireRequest {
        OwnerAcquireRequest {
            job_id: "job-1".to_string(),
            fence: crate::workflow::react::experiment_schedule::types::OwnerFence::new("harbor", 1),
            base_revision: "HEAD".to_string(),
            input_patch: None,
        }
    }

    #[cfg(unix)]
    fn write_manifest(path: &Path, capability: &HarborTaskCapabilityV1) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(
            path,
            serde_json::to_vec_pretty(capability).expect("serialize"),
        )
        .expect("write manifest");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .expect("restrict manifest");
    }

    #[cfg(not(unix))]
    fn write_manifest(path: &Path, capability: &HarborTaskCapabilityV1) {
        std::fs::write(
            path,
            serde_json::to_vec_pretty(capability).expect("serialize"),
        )
        .expect("write manifest");
    }

    #[test]
    fn without_a_capability_manifest_there_is_no_owner() {
        let directory = tempdir().expect("tempdir");
        let error = HarborTaskOwner::load(directory.path().join(HARBOR_CAPABILITY_FILE_NAME))
            .expect_err("missing manifest");
        assert_eq!(error.code, ScheduleErrorCode::OwnershipMismatch);
    }

    #[cfg(unix)]
    #[test]
    fn a_world_readable_manifest_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempdir().expect("tempdir");
        let manifest = directory.path().join(HARBOR_CAPABILITY_FILE_NAME);
        let capability = capability(directory.path(), &"a".repeat(64));
        std::fs::write(
            &manifest,
            serde_json::to_vec(&capability).expect("serialize"),
        )
        .expect("write");
        std::fs::set_permissions(&manifest, std::fs::Permissions::from_mode(0o644))
            .expect("widen manifest");
        let error = HarborTaskOwner::load(&manifest).expect_err("wide manifest");
        assert_eq!(error.code, ScheduleErrorCode::OwnershipMismatch);
    }

    #[test]
    fn a_malformed_manifest_is_refused() {
        let directory = tempdir().expect("tempdir");
        let manifest = directory.path().join(HARBOR_CAPABILITY_FILE_NAME);
        std::fs::write(&manifest, b"{ not json }").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&manifest, std::fs::Permissions::from_mode(0o600))
                .expect("restrict");
        }
        let error = HarborTaskOwner::load(&manifest).expect_err("malformed");
        assert_eq!(error.code, ScheduleErrorCode::UnsupportedOwnerKind);
    }

    #[test]
    fn preflight_requires_every_declared_root_to_exist() {
        let directory = tempdir().expect("tempdir");
        let mut capability = capability(directory.path(), &"a".repeat(64));
        let owner =
            HarborTaskOwner::from_capability(capability.clone(), directory.path()).expect("owner");
        owner.preflight().expect("all roots exist");

        capability.read_only_roots = vec![directory.path().join("absent").to_string_lossy().into()];
        let owner = HarborTaskOwner::from_capability(capability, directory.path()).expect("owner");
        let error = owner.preflight().expect_err("missing root");
        assert_eq!(error.code, ScheduleErrorCode::OwnershipMismatch);
    }

    #[test]
    fn the_owner_confines_patches_to_declared_roots_and_never_removes_the_task() {
        let directory = tempdir().expect("tempdir");
        let manifest = directory.path().join(HARBOR_CAPABILITY_FILE_NAME);
        let capability = capability(directory.path(), &"a".repeat(64));
        write_manifest(&manifest, &capability);
        let owner = HarborTaskOwner::load(&manifest).expect("load");
        assert_eq!(owner.kind(), OwnerKind::HarborTask);
        assert_eq!(owner.capability_path(), manifest.as_path());

        let workspace = owner.acquire(&request()).expect("acquire");
        assert_eq!(
            workspace.proof.workspace_root,
            PathBuf::from(&capability.workspace_root)
        );
        assert_eq!(
            workspace.proof.owner_token_hash,
            capability.owner_token_hash
        );

        // A proof from another token is refused.
        let mut foreign = workspace.clone();
        foreign.proof.owner_token_hash = "b".repeat(64);
        let error = owner
            .apply_input_patch(
                &foreign,
                &InputPatch {
                    bytes: b"x".to_vec(),
                    digest: patch::digest_hex(b"x"),
                },
            )
            .expect_err("foreign proof");
        assert_eq!(error.code, ScheduleErrorCode::OwnershipMismatch);

        // A workspace outside the declared roots can never be patched.
        let mut outside = workspace.clone();
        outside.proof.workspace_root = directory.path().join("elsewhere");
        let error = owner
            .apply_input_patch(
                &outside,
                &InputPatch {
                    bytes: b"x".to_vec(),
                    digest: patch::digest_hex(b"x"),
                },
            )
            .expect_err("outside the sandbox");
        assert_eq!(error.code, ScheduleErrorCode::WorkspaceEscape);

        // Real patch work inside the task workspace.
        init_workspace_repo(&workspace.proof.workspace_root);
        let bytes = b"--- a/app.py\n+++ b/app.py\n@@ -1 +1 @@\n-print('base')\n+print('patched')\n"
            .to_vec();
        owner
            .apply_input_patch(
                &workspace,
                &InputPatch {
                    bytes: bytes.clone(),
                    digest: patch::digest_hex(&bytes),
                },
            )
            .expect("apply inside the sandbox");
        std::fs::write(workspace.proof.workspace_root.join("result.txt"), "done\n")
            .expect("write result");

        // Publishing outside the declared artifact root fails closed.
        let outside_context = PatchContext {
            job_id: "job-1".to_string(),
            run_id: Some("run-1".to_string()),
            session_id: None,
            candidate_key: "baseline".to_string(),
            base_revision: "HEAD".to_string(),
            destination_root: directory.path().join("outside-artifacts"),
        };
        let error = owner
            .collect_output_patch(&workspace, &outside_context)
            .expect_err("outside artifact root");
        assert_eq!(error.code, ScheduleErrorCode::WorkspaceEscape);

        let context = PatchContext {
            destination_root: owner.artifact_root().expect("artifact root"),
            ..outside_context
        };
        let artifact = owner
            .collect_output_patch(&workspace, &context)
            .expect("collect inside the artifact root");
        assert!(artifact.directory.join("patch.diff").exists());
        let paths: Vec<&str> = artifact
            .manifest
            .files
            .iter()
            .map(|entry| entry.relative_path.as_str())
            .collect();
        assert!(paths.contains(&"app.py"), "paths: {paths:?}");
        assert!(paths.contains(&"result.txt"), "paths: {paths:?}");

        // Cleanup verifies ownership but never deletes the task environment.
        owner.cleanup(&workspace).expect("cleanup");
        assert!(Path::new(&capability.task_root).is_dir());
        assert!(Path::new(&capability.workspace_root).is_dir());
    }

    #[test]
    fn adoption_reuses_the_same_task_environment() {
        let directory = tempdir().expect("tempdir");
        let capability = capability(directory.path(), &"a".repeat(64));
        let owner =
            HarborTaskOwner::from_capability(capability.clone(), directory.path()).expect("owner");
        let adopted = owner
            .adopt(&request())
            .expect("adopt")
            .expect("the task sandbox is always adoptable");
        assert_eq!(
            adopted.proof.workspace_root,
            PathBuf::from(&capability.workspace_root)
        );
    }
}
