//! The filesystem-only execution owner: a run-scoped Git worktree.
//!
//! `HostWorktreeOwner` gives one job its own worktree at a pinned base revision,
//! applies the allowlisted input patch inside that worktree, and publishes the
//! collected output as an immutable patch artifact. It is intentionally the
//! *deterministic* owner: no shell is ever spawned, every `git` invocation is an
//! explicit argv, and the base repository's working tree, index and HEAD are
//! never written (AC-3/AC-5, INV-7).
//!
//! Ownership is carried by an on-disk marker that binds the worktree to the
//! job's `(owner_id, lease_generation)` token. Every later operation re-reads
//! that marker, so a superseded worker cannot adopt, mutate or clean up a newer
//! owner's worktree even though it still remembers the path (INV-8).
//!
//! This owner does not authorize host shell execution: a profile that selects
//! it can prepare and inspect a workspace, but the run itself must be dispatched
//! by an owner that provides a real execution environment (U-6).

use crate::workflow::react::experiment_owner::patch::{self, PatchArtifact, PatchContext};
use crate::workflow::react::experiment_owner::{
    owner_error, ExecutionOwner, InputPatch, OwnerAcquireRequest, PreparedWorkspace, WorkspaceProof,
};
use crate::workflow::react::experiment_schedule::types::{
    OwnerKind, ScheduleError, ScheduleErrorCode,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Marker file written inside every owned worktree.
pub const OWNER_MARKER_FILE: &str = ".cs-owner.json";

/// Fixed schema version of the ownership marker.
pub const OWNER_MARKER_V1: &str = "run_workspace_owner.v1";

/// The on-disk ownership marker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct WorkspaceOwnerMarkerV1 {
    pub schema_version: String,
    pub job_id: String,
    pub owner_token_hash: String,
    pub base_revision: String,
}

impl WorkspaceOwnerMarkerV1 {
    fn matches(&self, job_id: &str, token_hash: &str) -> bool {
        self.schema_version == OWNER_MARKER_V1
            && self.job_id == job_id
            && self.owner_token_hash == token_hash
    }
}

/// A Git-worktree execution owner.
pub struct HostWorktreeOwner {
    base_repo: PathBuf,
    worktrees_root: PathBuf,
}

impl HostWorktreeOwner {
    /// Binds the owner to a server-side base repository and worktrees root.
    ///
    /// Both paths come from the server configuration (the execution profile and
    /// the experiment domain), never from a caller or a candidate.
    pub fn new(base_repo: impl Into<PathBuf>, worktrees_root: impl Into<PathBuf>) -> Self {
        Self {
            base_repo: base_repo.into(),
            worktrees_root: worktrees_root.into(),
        }
    }

    /// The worktree path for one `(job, generation)` pair.
    fn workspace_path(&self, request: &OwnerAcquireRequest) -> Result<PathBuf, ScheduleError> {
        if !crate::workflow::react::experiment_schedule::types::is_valid_key(&request.job_id) {
            return Err(owner_error(
                ScheduleErrorCode::WorkspaceEscape,
                "the job id is not usable as a directory name",
            ));
        }
        Ok(self.worktrees_root.join(format!(
            "{}-g{}",
            request.job_id, request.fence.lease_generation
        )))
    }

    fn token_hash(&self, request: &OwnerAcquireRequest) -> String {
        request.fence.token_hash(&request.job_id)
    }

    fn marker_path(workspace: &Path) -> PathBuf {
        workspace.join(OWNER_MARKER_FILE)
    }

    fn read_marker(workspace: &Path) -> Result<Option<WorkspaceOwnerMarkerV1>, ScheduleError> {
        let path = Self::marker_path(workspace);
        let body = match std::fs::read(&path) {
            Ok(body) => body,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(owner_error(
                    ScheduleErrorCode::OwnershipMismatch,
                    format!("failed to read '{}': {error}", path.display()),
                ))
            }
        };
        serde_json::from_slice(&body).map(Some).map_err(|error| {
            owner_error(
                ScheduleErrorCode::OwnershipMismatch,
                format!(
                    "'{}' is not a valid ownership marker: {error}",
                    path.display()
                ),
            )
        })
    }

    fn write_marker(
        workspace: &Path,
        request: &OwnerAcquireRequest,
        token_hash: &str,
    ) -> Result<(), ScheduleError> {
        let marker = WorkspaceOwnerMarkerV1 {
            schema_version: OWNER_MARKER_V1.to_string(),
            job_id: request.job_id.clone(),
            owner_token_hash: token_hash.to_string(),
            base_revision: request.base_revision.clone(),
        };
        let body = serde_json::to_vec_pretty(&marker).map_err(|error| {
            owner_error(
                ScheduleErrorCode::OwnershipMismatch,
                format!("the ownership marker is not serializable: {error}"),
            )
        })?;
        std::fs::write(Self::marker_path(workspace), body).map_err(|error| {
            owner_error(
                ScheduleErrorCode::OwnershipMismatch,
                format!(
                    "failed to write the ownership marker in '{}': {error}",
                    workspace.display()
                ),
            )
        })
    }

    /// Verifies that `workspace` is owned by exactly this job generation.
    fn require_ownership(&self, workspace: &PreparedWorkspace) -> Result<(), ScheduleError> {
        let Some(marker) = Self::read_marker(&workspace.proof.workspace_root)? else {
            return Err(owner_error(
                ScheduleErrorCode::OwnershipMismatch,
                format!(
                    "the workspace '{}' carries no ownership marker",
                    workspace.proof.workspace_root.display()
                ),
            ));
        };
        if !marker.matches(&workspace.proof.job_id, &workspace.proof.owner_token_hash) {
            return Err(owner_error(
                ScheduleErrorCode::OwnershipMismatch,
                format!(
                    "the workspace '{}' belongs to another owner generation",
                    workspace.proof.workspace_root.display()
                ),
            ));
        }
        Ok(())
    }
}

impl ExecutionOwner for HostWorktreeOwner {
    fn kind(&self) -> OwnerKind {
        OwnerKind::HostWorktree
    }

    fn preflight(&self) -> Result<(), ScheduleError> {
        if !self.base_repo.is_dir() {
            return Err(owner_error(
                ScheduleErrorCode::ExecutorUnavailable,
                format!(
                    "the base repository '{}' does not exist",
                    self.base_repo.display()
                ),
            ));
        }
        // `git -C <repo> rev-parse --git-dir` proves both that git is available
        // and that the base repository really is a repository.
        patch::git_run(&self.base_repo, &["rev-parse", "--git-dir"], None).map(|_| ())
    }

    fn acquire(&self, request: &OwnerAcquireRequest) -> Result<PreparedWorkspace, ScheduleError> {
        self.preflight()?;
        let workspace = self.workspace_path(request)?;
        let token_hash = self.token_hash(request);

        if workspace.exists() {
            // Re-acquiring the same generation is idempotent; anything else is
            // another owner's resource and must never be touched.
            return match Self::read_marker(&workspace)? {
                Some(marker) if marker.matches(&request.job_id, &token_hash) => {
                    Ok(PreparedWorkspace {
                        proof: WorkspaceProof {
                            job_id: request.job_id.clone(),
                            owner_token_hash: token_hash,
                            base_revision: marker.base_revision,
                            workspace_root: workspace,
                            worktrees_root: self.worktrees_root.clone(),
                        },
                        input_patch_applied: false,
                        container: None,
                    })
                }
                _ => Err(owner_error(
                    ScheduleErrorCode::OwnershipMismatch,
                    format!(
                        "the workspace '{}' already exists and is not owned by this generation",
                        workspace.display()
                    ),
                )),
            };
        }

        std::fs::create_dir_all(&self.worktrees_root).map_err(|error| {
            owner_error(
                ScheduleErrorCode::ExecutorUnavailable,
                format!(
                    "failed to create '{}': {error}",
                    self.worktrees_root.display()
                ),
            )
        })?;
        let workspace_arg = workspace.to_string_lossy().to_string();
        patch::git_run(
            &self.base_repo,
            &[
                "worktree",
                "add",
                "--detach",
                workspace_arg.as_str(),
                request.base_revision.as_str(),
            ],
            None,
        )?;
        Self::write_marker(&workspace, request, &token_hash)?;

        let mut prepared = PreparedWorkspace {
            proof: WorkspaceProof {
                job_id: request.job_id.clone(),
                owner_token_hash: token_hash,
                base_revision: request.base_revision.clone(),
                workspace_root: workspace,
                worktrees_root: self.worktrees_root.clone(),
            },
            input_patch_applied: false,
            container: None,
        };
        // A declared input patch is applied as part of acquisition, so a run
        // never starts against a workspace whose input patch silently failed.
        if let Some(input_patch) = request.input_patch.as_ref() {
            self.apply_input_patch(&prepared, input_patch)?;
            prepared.input_patch_applied = true;
        }
        Ok(prepared)
    }

    fn adopt(
        &self,
        request: &OwnerAcquireRequest,
    ) -> Result<Option<PreparedWorkspace>, ScheduleError> {
        let workspace = self.workspace_path(request)?;
        if !workspace.exists() {
            return Ok(None);
        }
        let token_hash = self.token_hash(request);
        match Self::read_marker(&workspace)? {
            Some(marker) if marker.matches(&request.job_id, &token_hash) => {
                Ok(Some(PreparedWorkspace {
                    proof: WorkspaceProof {
                        job_id: request.job_id.clone(),
                        owner_token_hash: token_hash,
                        base_revision: marker.base_revision,
                        workspace_root: workspace,
                        worktrees_root: self.worktrees_root.clone(),
                    },
                    input_patch_applied: false,
                    container: None,
                }))
            }
            _ => Ok(None),
        }
    }

    fn apply_input_patch(
        &self,
        workspace: &PreparedWorkspace,
        patch: &InputPatch,
    ) -> Result<(), ScheduleError> {
        self.require_ownership(workspace)?;
        patch::apply_patch_in_workspace(&workspace.proof.workspace_root, patch)
    }

    fn collect_output_patch(
        &self,
        workspace: &PreparedWorkspace,
        context: &PatchContext,
    ) -> Result<PatchArtifact, ScheduleError> {
        self.require_ownership(workspace)?;
        patch::collect_workspace_patch(
            &workspace.proof.workspace_root,
            context,
            crate::headless::domain::now_ms(),
        )
    }

    fn cleanup(&self, workspace: &PreparedWorkspace) -> Result<(), ScheduleError> {
        let root = &workspace.proof.workspace_root;
        if !root.exists() {
            return Ok(());
        }
        // Only the owning generation may remove the worktree.
        self.require_ownership(workspace)?;
        let workspace_arg = root.to_string_lossy().to_string();
        patch::git_run(
            &self.base_repo,
            &["worktree", "remove", "--force", workspace_arg.as_str()],
            None,
        )?;
        let _ = patch::git_run(&self.base_repo, &["worktree", "prune"], None);
        if root.exists() {
            std::fs::remove_dir_all(root).map_err(|error| {
                owner_error(
                    ScheduleErrorCode::OwnershipMismatch,
                    format!("failed to remove '{}': {error}", root.display()),
                )
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::experiment_owner::patch::digest_hex;
    use crate::workflow::react::experiment_schedule::types::OwnerFence;
    use std::process::{Command, Stdio};
    use tempfile::tempdir;

    /// Creates a base repository with one committed file.
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
                .expect("run git");
            assert!(status.success(), "git {args:?} failed");
        };
        run(&["init", "--quiet"]);
        run(&["config", "user.email", "cs-test@example.invalid"]);
        run(&["config", "user.name", "cs-test"]);
        std::fs::write(repo.join("README.md"), "base\n").expect("write readme");
        std::fs::create_dir_all(repo.join("src")).expect("create src");
        std::fs::write(repo.join("src/lib.rs"), "pub fn base() {}\n").expect("write lib");
        run(&["add", "-A"]);
        run(&["commit", "--quiet", "-m", "base"]);
        repo
    }

    fn owner(directory: &Path) -> (HostWorktreeOwner, PathBuf) {
        let repo = base_repo(directory);
        let worktrees = directory.join("worktrees");
        (HostWorktreeOwner::new(&repo, &worktrees), repo)
    }

    fn request(owner_token: &str, generation: i64) -> OwnerAcquireRequest {
        OwnerAcquireRequest {
            job_id: "job-1".to_string(),
            fence: OwnerFence::new(owner_token, generation),
            base_revision: "HEAD".to_string(),
            input_patch: None,
        }
    }

    fn patch_context(directory: &Path) -> PatchContext {
        PatchContext {
            job_id: "job-1".to_string(),
            run_id: Some("run-1".to_string()),
            session_id: Some("session-1".to_string()),
            candidate_key: "baseline".to_string(),
            base_revision: "HEAD".to_string(),
            destination_root: directory.join("artifacts"),
        }
    }

    /// Snapshot of the base repository's read-only state.
    fn base_state(repo: &Path) -> (String, String, String) {
        let capture = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(args)
                .output()
                .expect("git");
            String::from_utf8_lossy(&output.stdout).to_string()
        };
        (
            capture(&["rev-parse", "HEAD"]),
            capture(&["status", "--porcelain"]),
            capture(&["diff", "--stat"]),
        )
    }

    #[test]
    fn preflight_reports_a_missing_repository() {
        let directory = tempdir().expect("tempdir");
        let owner = HostWorktreeOwner::new(
            directory.path().join("absent"),
            directory.path().join("worktrees"),
        );
        let error = owner.preflight().expect_err("missing repo");
        assert_eq!(error.code, ScheduleErrorCode::ExecutorUnavailable);
    }

    #[test]
    fn acquire_applies_a_patch_and_publishes_output_without_touching_the_base_tree() {
        let directory = tempdir().expect("tempdir");
        let (owner, repo) = owner(directory.path());
        let before = base_state(&repo);
        owner.preflight().expect("preflight");

        let workspace = owner.acquire(&request("worker-a", 1)).expect("acquire");
        assert!(workspace.proof.workspace_root.join("src/lib.rs").exists());
        assert_eq!(workspace.proof.owner_token_hash.len(), 64);
        assert_eq!(
            base_state(&repo).1,
            before.1,
            "the base working tree must stay clean"
        );

        // Apply an allowlisted input patch inside the worktree.
        let patch_bytes =
            b"--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-pub fn base() {}\n+pub fn base() { /* input */ }\n"
                .to_vec();
        let input = InputPatch {
            digest: digest_hex(&patch_bytes),
            bytes: patch_bytes,
        };
        owner
            .apply_input_patch(&workspace, &input)
            .expect("apply input patch");
        let applied = std::fs::read_to_string(workspace.proof.workspace_root.join("src/lib.rs"))
            .expect("read worktree file");
        assert!(applied.contains("/* input */"));
        // The base tree is still untouched by the input patch.
        assert_eq!(base_state(&repo), before);

        // The run adds an untracked file and modifies another.
        std::fs::write(
            workspace.proof.workspace_root.join("src/added.rs"),
            "pub fn added() {}\n",
        )
        .expect("write added");
        std::fs::write(
            workspace.proof.workspace_root.join("README.md"),
            "base\nmodified\n",
        )
        .expect("write readme");

        let artifact = owner
            .collect_output_patch(&workspace, &patch_context(directory.path()))
            .expect("collect");
        assert!(artifact.directory.join("patch.diff").exists());
        assert_eq!(artifact.relative_path, "job-1/patch.diff");
        assert_eq!(artifact.manifest.job_id, "job-1");
        assert_eq!(artifact.manifest.run_id.as_deref(), Some("run-1"));
        let paths: Vec<&str> = artifact
            .manifest
            .files
            .iter()
            .map(|entry| entry.relative_path.as_str())
            .collect();
        assert!(paths.contains(&"src/added.rs"), "paths: {paths:?}");
        assert!(paths.contains(&"README.md"), "paths: {paths:?}");
        assert_eq!(artifact.manifest.diff_sha256, artifact.sha256);

        // The base repository is still exactly as it was.
        assert_eq!(base_state(&repo), before);

        owner.cleanup(&workspace).expect("cleanup");
        assert!(!workspace.proof.workspace_root.exists());
        assert_eq!(base_state(&repo), before);
        // Cleanup is idempotent.
        owner.cleanup(&workspace).expect("cleanup again");
    }

    #[test]
    fn adopt_matches_only_the_owning_generation() {
        let directory = tempdir().expect("tempdir");
        let (owner, _repo) = owner(directory.path());
        let workspace = owner.acquire(&request("worker-a", 1)).expect("acquire");

        let adopted = owner
            .adopt(&request("worker-a", 1))
            .expect("adopt")
            .expect("same generation is adoptable");
        assert_eq!(adopted.proof.workspace_root, workspace.proof.workspace_root);

        // A different generation (and a different owner) never adopts it.
        assert!(owner
            .adopt(&request("worker-a", 2))
            .expect("adopt")
            .is_none());
        assert!(owner
            .adopt(&request("worker-b", 1))
            .expect("adopt")
            .is_none());

        // The same generation number belonging to another owner maps to the
        // same path, and taking that path over fails closed.
        let error = owner
            .acquire(&request("worker-b", 1))
            .expect_err("must not take over another owner's workspace");
        assert_eq!(error.code, ScheduleErrorCode::OwnershipMismatch);
        // A later generation of the same job gets its own fresh workspace.
        let next = owner
            .acquire(&request("worker-a", 2))
            .expect("a new generation acquires its own workspace");
        assert_ne!(next.proof.workspace_root, workspace.proof.workspace_root);
    }

    #[test]
    fn a_wrong_or_unappliable_input_patch_is_rejected() {
        let directory = tempdir().expect("tempdir");
        let (owner, repo) = owner(directory.path());
        let workspace = owner.acquire(&request("worker-a", 1)).expect("acquire");
        let before = base_state(&repo);

        // Digest mismatch.
        let error = owner
            .apply_input_patch(
                &workspace,
                &InputPatch {
                    bytes: b"x".to_vec(),
                    digest: "0".repeat(64),
                },
            )
            .expect_err("digest mismatch");
        assert_eq!(error.code, ScheduleErrorCode::InputPatchRejected);

        // A patch that does not apply to this base revision.
        let bytes = b"--- a/nope.txt\n+++ b/nope.txt\n@@ -1 +1 @@\n-a\n+b\n".to_vec();
        let error = owner
            .apply_input_patch(
                &workspace,
                &InputPatch {
                    digest: digest_hex(&bytes),
                    bytes,
                },
            )
            .expect_err("unappliable patch");
        assert_eq!(error.code, ScheduleErrorCode::InputPatchRejected);
        assert_eq!(base_state(&repo), before);
    }

    #[test]
    fn a_symlink_created_by_the_run_is_not_publishable() {
        let directory = tempdir().expect("tempdir");
        let (owner, _repo) = owner(directory.path());
        let workspace = owner.acquire(&request("worker-a", 1)).expect("acquire");
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/passwd", workspace.proof.workspace_root.join("leak"))
            .expect("symlink");
        #[cfg(not(unix))]
        return;

        let error = owner
            .collect_output_patch(&workspace, &patch_context(directory.path()))
            .expect_err("symlink must be rejected");
        assert_eq!(error.code, ScheduleErrorCode::WorkspaceEscape);
        assert!(!directory.path().join("artifacts").join("job-1").exists());
    }

    #[test]
    fn a_superseded_owner_cannot_mutate_or_remove_the_workspace() {
        let directory = tempdir().expect("tempdir");
        let (owner, _repo) = owner(directory.path());
        let workspace = owner.acquire(&request("worker-a", 1)).expect("acquire");

        // The same path remembered by a superseded generation.
        let stale = PreparedWorkspace {
            proof: WorkspaceProof {
                job_id: "job-1".to_string(),
                owner_token_hash: OwnerFence::new("worker-a", 2).token_hash("job-1"),
                base_revision: "HEAD".to_string(),
                workspace_root: workspace.proof.workspace_root.clone(),
                worktrees_root: workspace.proof.worktrees_root.clone(),
            },
            input_patch_applied: false,
            container: None,
        };
        let bytes = b"--- a/README.md\n+++ b/README.md\n@@ -1 +1 @@\n-base\n+stale\n".to_vec();
        let error = owner
            .apply_input_patch(
                &stale,
                &InputPatch {
                    digest: digest_hex(&bytes),
                    bytes,
                },
            )
            .expect_err("stale generation must not patch");
        assert_eq!(error.code, ScheduleErrorCode::OwnershipMismatch);
        let error = owner
            .cleanup(&stale)
            .expect_err("stale generation must not clean up");
        assert_eq!(error.code, ScheduleErrorCode::OwnershipMismatch);
        assert!(workspace.proof.workspace_root.exists());
    }
}
