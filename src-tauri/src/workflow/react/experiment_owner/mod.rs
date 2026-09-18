//! Phase 2G isolated execution owner.
//!
//! An *execution owner* owns exactly one run's mutable execution environment:
//! the run worktree, the allowlisted input patch applied inside it, the
//! immutable output patch it publishes, and the cleanup that rolls all of it
//! back. The owner is the only component allowed to mutate that environment,
//! and it is fenced by the job's `(owner_id, lease_generation)` token so a
//! superseded worker can never touch a newer owner's resources (INV-8).
//!
//! Two owner kinds are planned; this module delivers the shared contract plus
//! [`HostWorktreeOwner`], the filesystem-only owner:
//!
//! - [`HostWorktreeOwner`] prepares a run-scoped Git worktree, applies an
//!   allowlisted input patch inside it, and publishes the output as an
//!   immutable `patch.diff` + manifest. It never executes a shell, never
//!   touches the base repository's working tree, and never authorizes host
//!   execution: it is the deterministic, cross-platform owner used by focused
//!   tests.
//! - [`PersistentDockerOwner`] owns a label-fenced, digest-pinned container that
//!   executes the run against the worktree mounted as its only writable path.
//! - [`HarborTaskOwner`] proves ownership of a Harbor task environment from the
//!   adapter-written capability manifest, so the same scheduler drives the real
//!   Harbor isolation gate without a second lifecycle.
//!
//! All three implement the same trait, so the scheduler never learns a second
//! lifecycle.
//!
//! Hard rules enforced here (AC-3/AC-5, INV-4/INV-6/INV-7/INV-8):
//!
//! - Every path the owner touches is derived from the *server-side* base repo
//!   and worktrees root; a caller or candidate can never supply a host path.
//! - The base repository's working tree, index and HEAD are strictly read-only.
//! - The output patch is scanned for credentials before it is published, and a
//!   hit refuses publication instead of writing a redacted-but-silently-wrong
//!   artifact.
//! - There is no apply/merge-to-main API anywhere in this module.

pub mod bundle;
pub mod capabilities;
pub mod docker;
pub mod harbor_task;
pub mod patch;
pub mod promotion;
pub mod promotion_canary;
pub mod worktree;

use crate::workflow::react::experiment_owner::patch::{PatchArtifact, PatchContext};
use crate::workflow::react::experiment_schedule::types::{
    OwnerFence, OwnerKind, ScheduleError, ScheduleErrorCode,
};
use std::path::PathBuf;

/// An allowlisted input patch, already digest-bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputPatch {
    pub bytes: Vec<u8>,
    /// Lowercase hex sha256 of `bytes`; the owner re-checks it before applying.
    pub digest: String,
}

/// Everything the owner needs to acquire one run's environment.
#[derive(Debug, Clone)]
pub struct OwnerAcquireRequest {
    pub job_id: String,
    pub fence: OwnerFence,
    /// Immutable server-side base revision the worktree is created at.
    pub base_revision: String,
    /// The reviewed input patch, when the execution profile declares one.
    pub input_patch: Option<InputPatch>,
    /// Server-derived root for this job's verified bundle staging tree. It is
    /// never caller supplied and is mounted read-only only by owners whose
    /// profile explicitly declares the bundle mount.
    pub bundle_source_root: Option<PathBuf>,
}

/// Proof that one owner generation owns a workspace.
///
/// It is produced only by an owner and is required by every later operation, so
/// a worker that lost its lease cannot keep mutating the environment by simply
/// remembering a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceProof {
    pub job_id: String,
    pub owner_token_hash: String,
    pub base_revision: String,
    pub workspace_root: PathBuf,
    pub worktrees_root: PathBuf,
}

impl WorkspaceProof {
    /// The workspace path including the run's base revision.
    pub fn describe(&self) -> String {
        format!(
            "job={} base_revision={} workspace={}",
            self.job_id,
            self.base_revision,
            self.workspace_root.display()
        )
    }
}

/// A read-only verified bundle mount belonging to a container owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleMount {
    pub host_root: PathBuf,
    pub container_root: PathBuf,
}

/// The container an owner created or adopted, when the owner provides one.
///
/// It is part of the ownership proof: cleanup and adoption both compare the
/// recorded labels/token before touching anything, so an owner can never act on
/// a container it does not own (INV-8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerHandle {
    pub name: String,
    pub owner_token_hash: String,
    pub image_reference: String,
    /// The verified bundle tree visible to this owner, when its profile
    /// declared the dedicated read-only bundle mount.
    pub bundle_mount: Option<BundleMount>,
}

/// A prepared, owned run workspace.
#[derive(Debug, Clone)]
pub struct PreparedWorkspace {
    pub proof: WorkspaceProof,
    pub input_patch_applied: bool,
    /// The isolated execution environment, when this owner provides one.
    pub container: Option<ContainerHandle>,
}

/// The owner contract shared by every isolation implementation.
///
/// All methods are synchronous filesystem/process work; the scheduler drives
/// them from its own bounded task, so the trait stays free of runtime
/// assumptions.
pub trait ExecutionOwner: Send + Sync {
    /// The owner kind this implementation provides.
    fn kind(&self) -> OwnerKind;

    /// Fails closed when the environment the owner needs is unavailable.
    ///
    /// It is called before any run is dispatched, so an unavailable runtime is
    /// a pre-dispatch `failed_precondition` rather than a silent fallback
    /// (INV-4).
    fn preflight(&self) -> Result<(), ScheduleError>;

    /// Acquires the environment for one job, or fails closed.
    fn acquire(&self, request: &OwnerAcquireRequest) -> Result<PreparedWorkspace, ScheduleError>;

    /// Adopts an environment this same owner generation already created.
    ///
    /// Used by recovery: it returns `None` when nothing adoptable exists, and
    /// never adopts another generation's resources.
    fn adopt(
        &self,
        request: &OwnerAcquireRequest,
    ) -> Result<Option<PreparedWorkspace>, ScheduleError>;

    /// Applies the allowlisted input patch inside the owned workspace only.
    fn apply_input_patch(
        &self,
        workspace: &PreparedWorkspace,
        patch: &InputPatch,
    ) -> Result<(), ScheduleError>;

    /// Where this owner allows evidence to be published.
    ///
    /// `None` means "the scheduler's own artifact root". An owner that confines
    /// publication to a root it declared (the Harbor task environment, whose
    /// artifact root is `/logs/artifacts`) returns it here, so the run's patch
    /// is never written outside the boundary the owner proved.
    fn artifact_root(&self) -> Option<std::path::PathBuf> {
        None
    }

    /// Publishes the immutable output patch and its manifest.
    fn collect_output_patch(
        &self,
        workspace: &PreparedWorkspace,
        context: &PatchContext,
    ) -> Result<PatchArtifact, ScheduleError>;

    /// Idempotently removes everything this owner generation created.
    fn cleanup(&self, workspace: &PreparedWorkspace) -> Result<(), ScheduleError>;
}

/// Builds an owner error with a stable machine code.
pub fn owner_error(code: ScheduleErrorCode, message: impl Into<String>) -> ScheduleError {
    ScheduleError::new(code, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_owner_error_carries_a_stable_code() {
        let error = owner_error(ScheduleErrorCode::WorkspaceEscape, "escape");
        assert_eq!(error.code.as_str(), "workspace_escape");
        assert_eq!(error.message, "escape");
    }

    #[test]
    fn a_workspace_proof_renders_its_identity_without_secrets() {
        let proof = WorkspaceProof {
            job_id: "job-1".to_string(),
            owner_token_hash: "a".repeat(64),
            base_revision: "refs/heads/main".to_string(),
            workspace_root: PathBuf::from("/tmp/ws"),
            worktrees_root: PathBuf::from("/tmp"),
        };
        let rendered = proof.describe();
        assert!(rendered.contains("job-1"));
        assert!(rendered.contains("refs/heads/main"));
        assert!(!rendered.contains(&"a".repeat(64)));
    }
}
