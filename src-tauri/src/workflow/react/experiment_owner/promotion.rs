//! The Phase 2I promotion checkpoint owner: the only component that mutates a
//! persistent Git reference.
//!
//! It is deliberately **separate** from the run-scoped [`ExecutionOwner`]. A
//! normal job owner owns a workspace, a container or a task sandbox for the
//! lifetime of one run and then destroys it; it never touches a branch. This
//! owner owns *persistent* state:
//!
//! - a detached worktree derived from the expected old head,
//! - one backend-minted checkpoint commit and its namespaced ref
//!   (`refs/chatspeed/checkpoints/<promotion_id>`), and
//! - the registered local experiment branch, advanced only by an
//!   expected-old compare-and-swap.
//!
//! Invariants enforced here (AC-5/AC-7; INV-4/5/6/7/8):
//!
//! - Every Git invocation is explicit argv. There is no shell, and an argv
//!   allowlist refuses `push`, `fetch`, `pull`, `remote`, `merge`, `rebase`,
//!   `cherry-pick`, `reset`, `clone`, `submodule` and friends outright, so this
//!   module can never reach a remote or rewrite history.
//! - The base repository's worktree, index and HEAD are never modified: the
//!   only `add`/`commit` runs inside the detached worktree.
//! - The checkpoint commit identity is supplied per command (`-c user.name`,
//!   `-c user.email`, `--no-verify`); the user's Git configuration is never
//!   written.
//! - `observe_*` never mutates: recovery classifies from observation only.
//! - Cleanup removes the worktree and never the checkpoint ref, so a failure
//!   keeps its evidence.
//!
//! [`ExecutionOwner`]: super::ExecutionOwner

use crate::workflow::react::experiment_owner::patch::digest_hex;
use crate::workflow::react::experiment_promotion::types::{
    checkpoint_ref_for, is_full_branch_ref, is_sha256_hex, validate_promotion_id,
    BranchObservation, CheckpointObservation, PromotionError, PromotionErrorCode, PromotionFence,
};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn promotion_error(code: PromotionErrorCode, message: impl Into<String>) -> PromotionError {
    PromotionError::new(code, message)
}

/// The scope of one promotion effect: the promotion identity, its fence and the
/// server-owned target facts it may act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionCheckpointRequest {
    pub promotion_id: String,
    pub fence: PromotionFence,
    /// The full `refs/heads/*` experiment branch this attempt may advance.
    pub branch_ref: String,
    /// The head the branch must still carry before any effect.
    pub expected_old_head: String,
    /// The base revision the patch was produced against.
    pub base_revision: String,
    pub git_identity_name: String,
    pub git_identity_email: String,
    /// The registered target reference, recorded in the commit trailers.
    pub target_ref: String,
    /// The canonical evidence hash, recorded in the commit trailers.
    pub evidence_hash: String,
    /// The digest the patch bytes must match.
    pub patch_sha256: String,
}

/// Proof that one checkpoint commit and ref exist for this attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointProof {
    pub promotion_id: String,
    pub checkpoint_commit: String,
    pub checkpoint_ref: String,
    pub workspace_root: PathBuf,
}

/// Subcommands this owner may ever run. Anything else — in particular any
/// command that could talk to a remote or rewrite history — is refused before a
/// process is spawned.
const ALLOWED_SUBCOMMANDS: &[&str] = &[
    "rev-parse",
    "worktree",
    "apply",
    "add",
    "commit",
    "update-ref",
    "log",
    "status",
    "diff",
    "show",
    "cat-file",
    "ls-files",
];

/// Tokens that must never appear in an argv this owner builds. The allowlist
/// above is the primary control; this list is a second, explicit guard so a
/// future edit cannot quietly introduce a remote or history-rewriting command.
const FORBIDDEN_TOKENS: &[&str] = &[
    "push",
    "fetch",
    "pull",
    "remote",
    "merge",
    "rebase",
    "cherry-pick",
    "revert",
    "reset",
    "clone",
    "submodule",
    "filter-branch",
    "gc",
    "prune",
    "daemon",
    "http-fetch",
    "send-pack",
    "receive-pack",
    "replace",
    "notes",
];

/// The promotion checkpoint owner.
#[derive(Debug, Clone)]
pub struct PromotionCheckpointOwner {
    base_repo: PathBuf,
    worktrees_root: PathBuf,
}

impl PromotionCheckpointOwner {
    /// Binds the owner to the server-side base repository and the experiment
    /// domain's worktrees root. Both come from server configuration, never from
    /// a caller or a candidate.
    pub fn new(base_repo: impl Into<PathBuf>, worktrees_root: impl Into<PathBuf>) -> Self {
        Self {
            base_repo: base_repo.into(),
            worktrees_root: worktrees_root.into(),
        }
    }

    pub fn base_repo(&self) -> &Path {
        &self.base_repo
    }

    /// The detached worktree path for one `(promotion, generation)` pair. The
    /// generation is part of the name, so a superseded worker can never reuse
    /// or destroy its successor's workspace.
    fn workspace_path(
        &self,
        request: &PromotionCheckpointRequest,
    ) -> Result<PathBuf, PromotionError> {
        validate_promotion_id(&request.promotion_id)?;
        Ok(self.worktrees_root.join(format!(
            "{}-g{}",
            request.promotion_id, request.fence.lease_generation
        )))
    }

    /// Proves the base repository exists and is a repository, and that the
    /// target branch is a full `refs/heads/*` ref.
    pub fn preflight(&self, request: &PromotionCheckpointRequest) -> Result<(), PromotionError> {
        if !self.base_repo.is_dir() {
            return Err(promotion_error(
                PromotionErrorCode::RepositoryUnavailable,
                format!(
                    "the base repository '{}' does not exist",
                    self.base_repo.display()
                ),
            ));
        }
        if !is_full_branch_ref(&request.branch_ref) {
            return Err(promotion_error(
                PromotionErrorCode::UnsafeTargetRef,
                format!("'{}' is not a full refs/heads/* branch", request.branch_ref),
            ));
        }
        run_git(&self.base_repo, &["rev-parse", "--git-dir"], None).map_err(|error| {
            promotion_error(
                PromotionErrorCode::RepositoryUnavailable,
                format!(
                    "'{}' is not a usable Git repository: {}",
                    self.base_repo.display(),
                    error.message
                ),
            )
        })?;
        Ok(())
    }

    /// Reads the current head of the registered branch.
    pub fn observe_branch_head(&self, branch_ref: &str) -> Result<String, PromotionError> {
        if !is_full_branch_ref(branch_ref) {
            return Err(promotion_error(
                PromotionErrorCode::UnsafeTargetRef,
                format!("'{branch_ref}' is not a full refs/heads/* branch"),
            ));
        }
        let output = run_git(
            &self.base_repo,
            &["rev-parse", "--verify", "--quiet", branch_ref],
            None,
        )?;
        let head = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if head.is_empty() {
            return Err(promotion_error(
                PromotionErrorCode::RepositoryUnavailable,
                format!("the registered branch '{branch_ref}' does not resolve"),
            ));
        }
        Ok(head)
    }

    /// Classifies the registered branch against the expected old head and this
    /// attempt's checkpoint commit. Pure observation: nothing is written.
    pub fn observe_branch(
        &self,
        branch_ref: &str,
        expected_old_head: &str,
        checkpoint_commit: &str,
    ) -> Result<BranchObservation, PromotionError> {
        let head = self.observe_branch_head(branch_ref)?;
        Ok(if head == expected_old_head {
            BranchObservation::AtOld
        } else if head == checkpoint_commit {
            BranchObservation::AtCheckpoint
        } else {
            BranchObservation::ThirdValue
        })
    }

    /// Classifies the namespaced checkpoint ref of this attempt.
    ///
    /// The ref is only adopted when its commit carries the trailers this
    /// attempt expects, so a foreign or drifted ref is never mistaken for our
    /// own effect.
    pub fn observe_checkpoint(
        &self,
        request: &PromotionCheckpointRequest,
    ) -> Result<CheckpointObservation, PromotionError> {
        let checkpoint_ref = checkpoint_ref_for(&request.promotion_id);
        let output = run_git(
            &self.base_repo,
            &["rev-parse", "--verify", "--quiet", &checkpoint_ref],
            None,
        )?;
        if !output.status.success() {
            return Ok(CheckpointObservation::Absent);
        }
        let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if commit.is_empty() {
            return Ok(CheckpointObservation::Absent);
        }
        let body = run_git(
            &self.base_repo,
            &["log", "-1", "--format=%B", &checkpoint_ref],
            None,
        )?;
        let body = String::from_utf8_lossy(&body.stdout).to_string();
        let consistent = trailer(&body, "Promotion-Id").as_deref()
            == Some(request.promotion_id.as_str())
            && trailer(&body, "Evidence-Hash").as_deref() == Some(request.evidence_hash.as_str())
            && trailer(&body, "Patch-Sha256").as_deref() == Some(request.patch_sha256.as_str())
            && trailer(&body, "Base-Revision").as_deref() == Some(request.base_revision.as_str())
            && trailer(&body, "Target-Ref").as_deref() == Some(request.target_ref.as_str());
        Ok(if consistent {
            CheckpointObservation::PresentConsistent
        } else {
            CheckpointObservation::PresentInconsistent
        })
    }

    /// The commit the namespaced checkpoint ref currently points at, if the ref
    /// exists. Read-only: adoption uses it to learn the commit an earlier
    /// attempt already published.
    pub fn checkpoint_commit(
        &self,
        request: &PromotionCheckpointRequest,
    ) -> Result<Option<String>, PromotionError> {
        let checkpoint_ref = checkpoint_ref_for(&request.promotion_id);
        let output = run_git(
            &self.base_repo,
            &["rev-parse", "--verify", "--quiet", &checkpoint_ref],
            None,
        )?;
        if !output.status.success() {
            return Ok(None);
        }
        let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok((!commit.is_empty()).then_some(commit))
    }

    /// Creates the detached worktree, applies the digest-bound patch and
    /// publishes the checkpoint commit and ref.
    ///
    /// The method is *idempotent for this attempt*: when the checkpoint ref
    /// already exists and is consistent, it returns the existing proof instead
    /// of creating a second commit. Any other pre-existing ref state is refused.
    pub fn create_checkpoint(
        &self,
        request: &PromotionCheckpointRequest,
        patch_bytes: &[u8],
    ) -> Result<CheckpointProof, PromotionError> {
        self.preflight(request)?;
        if !is_sha256_hex(&request.patch_sha256) {
            return Err(promotion_error(
                PromotionErrorCode::MalformedDigest,
                "the patch digest is not a sha256 hex digest",
            ));
        }
        if digest_hex(patch_bytes) != request.patch_sha256 {
            return Err(promotion_error(
                PromotionErrorCode::PatchApplyFailed,
                "the patch bytes do not match the digest the promotion is bound to",
            ));
        }

        // Adoption: the effect provably already happened for this attempt.
        let checkpoint_ref = checkpoint_ref_for(&request.promotion_id);
        if self.observe_checkpoint(request)? == CheckpointObservation::PresentConsistent {
            let commit = run_git(
                &self.base_repo,
                &["rev-parse", "--verify", &checkpoint_ref],
                None,
            )?;
            return Ok(CheckpointProof {
                promotion_id: request.promotion_id.clone(),
                checkpoint_commit: String::from_utf8_lossy(&commit.stdout).trim().to_string(),
                checkpoint_ref,
                workspace_root: self.workspace_path(request)?,
            });
        }

        // The expected old head must still be the branch head, and the branch
        // must not be checked out anywhere: `update-ref` would move a branch a
        // user has checked out from under them.
        let head = self.observe_branch_head(&request.branch_ref)?;
        if head != request.expected_old_head {
            return Err(promotion_error(
                PromotionErrorCode::BranchHeadDrift,
                format!(
                    "branch '{}' is at {head}, the promotion expected {}",
                    request.branch_ref, request.expected_old_head
                ),
            ));
        }
        self.require_branch_not_checked_out(&request.branch_ref)?;

        let workspace = self.workspace_path(request)?;
        std::fs::create_dir_all(&self.worktrees_root).map_err(|error| {
            promotion_error(
                PromotionErrorCode::CheckpointFailed,
                format!(
                    "failed to create '{}': {error}",
                    self.worktrees_root.display()
                ),
            )
        })?;
        if workspace.exists() {
            return Err(promotion_error(
                PromotionErrorCode::CheckpointFailed,
                format!(
                    "the promotion workspace '{}' already exists",
                    workspace.display()
                ),
            ));
        }
        // Reclaim the checkpoint worktree a previous generation may have left
        // behind when it was killed mid-effect. The prefix is this promotion's
        // own id (never a caller string), and only this promotion's worktrees
        // under the domain root match it, so the reclamation is ownership-proven.
        self.remove_stale_worktrees(&format!("{}-g", request.promotion_id))?;
        for entry in std::fs::read_dir(&self.worktrees_root)
            .map_err(|error| {
                promotion_error(
                    PromotionErrorCode::CheckpointFailed,
                    format!(
                        "failed to read '{}': {error}",
                        self.worktrees_root.display()
                    ),
                )
            })?
            .flatten()
        {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(&format!("{}-g", request.promotion_id)) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
        let workspace_arg = workspace.to_string_lossy().to_string();
        git_ok(
            &self.base_repo,
            &[
                "worktree",
                "add",
                "--detach",
                &workspace_arg,
                &request.expected_old_head,
            ],
            None,
        )
        .map_err(|error| {
            promotion_error(
                PromotionErrorCode::CheckpointFailed,
                format!("failed to create the promotion worktree: {}", error.message),
            )
        })?;

        // Everything below happens inside the detached worktree only.
        let result = self.create_checkpoint_in_workspace(request, patch_bytes, &workspace);
        if result.is_err() {
            // Never leave an unowned worktree behind on a failed checkpoint; the
            // checkpoint ref (if it had been created) is deliberately not
            // touched.
            let _ = run_git(
                &self.base_repo,
                &["worktree", "remove", "--force", &workspace_arg],
                None,
            );
        }
        result
    }

    fn create_checkpoint_in_workspace(
        &self,
        request: &PromotionCheckpointRequest,
        patch_bytes: &[u8],
        workspace: &Path,
    ) -> Result<CheckpointProof, PromotionError> {
        // `--check` first: a patch that does not apply cleanly is refused before
        // anything is written.
        git_ok(workspace, &["apply", "--check", "-"], Some(patch_bytes)).map_err(|error| {
            promotion_error(
                PromotionErrorCode::PatchApplyFailed,
                format!("the patch does not apply cleanly: {}", error.message),
            )
        })?;
        git_ok(workspace, &["apply", "-"], Some(patch_bytes)).map_err(|error| {
            promotion_error(
                PromotionErrorCode::PatchApplyFailed,
                format!("the patch failed to apply: {}", error.message),
            )
        })?;
        git_ok(workspace, &["add", "-A"], None).map_err(|error| {
            promotion_error(
                PromotionErrorCode::CheckpointFailed,
                format!("failed to stage the checkpoint: {}", error.message),
            )
        })?;
        let staged = git_ok(workspace, &["status", "--porcelain"], None)?;
        if staged.stdout.is_empty() {
            return Err(promotion_error(
                PromotionErrorCode::CheckpointFailed,
                "the patch produced no changes to checkpoint",
            ));
        }

        let subject = format!("experiment(promotion): checkpoint {}", request.promotion_id);
        let body = format!(
            "Promotion-Id: {}\nEvidence-Hash: {}\nPatch-Sha256: {}\nBase-Revision: {}\nTarget-Ref: {}",
            request.promotion_id,
            request.evidence_hash,
            request.patch_sha256,
            request.base_revision,
            request.target_ref
        );
        let name_arg = format!("user.name={}", request.git_identity_name);
        let email_arg = format!("user.email={}", request.git_identity_email);
        git_ok(
            workspace,
            &[
                "-c",
                &name_arg,
                "-c",
                &email_arg,
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--no-verify",
                "-m",
                &subject,
                "-m",
                &body,
            ],
            None,
        )
        .map_err(|error| {
            promotion_error(
                PromotionErrorCode::CheckpointFailed,
                format!("failed to create the checkpoint commit: {}", error.message),
            )
        })?;

        let commit = git_ok(workspace, &["rev-parse", "HEAD"], None)?;
        let commit = String::from_utf8_lossy(&commit.stdout).trim().to_string();
        if commit.len() < 40 || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(promotion_error(
                PromotionErrorCode::CheckpointFailed,
                "the checkpoint commit id could not be read",
            ));
        }

        let checkpoint_ref = checkpoint_ref_for(&request.promotion_id);
        // `""` as the old value requires the ref to not exist, so a concurrent
        // creator is never overwritten.
        git_ok(
            &self.base_repo,
            &["update-ref", &checkpoint_ref, &commit, ""],
            None,
        )
        .map_err(|error| {
            promotion_error(
                PromotionErrorCode::CheckpointFailed,
                format!(
                    "failed to create the checkpoint ref '{checkpoint_ref}': {}",
                    error.message
                ),
            )
        })?;

        Ok(CheckpointProof {
            promotion_id: request.promotion_id.clone(),
            checkpoint_commit: commit,
            checkpoint_ref,
            workspace_root: workspace.to_path_buf(),
        })
    }

    /// Advances the registered branch from the expected old head to the
    /// checkpoint commit with an expected-old compare-and-swap.
    ///
    /// A concurrently moved branch matches zero rows and surfaces as a drift
    /// error; the caller then re-observes instead of forcing the update.
    pub fn advance_branch(
        &self,
        request: &PromotionCheckpointRequest,
        checkpoint_commit: &str,
    ) -> Result<BranchObservation, PromotionError> {
        self.preflight(request)?;
        let observed = self.observe_branch(
            &request.branch_ref,
            &request.expected_old_head,
            checkpoint_commit,
        )?;
        // The CAS is idempotent for the two states that prove our own effect.
        if observed == BranchObservation::AtCheckpoint {
            return Ok(observed);
        }
        if observed == BranchObservation::ThirdValue {
            return Err(promotion_error(
                PromotionErrorCode::BranchHeadDrift,
                format!(
                    "branch '{}' moved away from both the expected old head and the checkpoint",
                    request.branch_ref
                ),
            ));
        }
        git_ok(
            &self.base_repo,
            &[
                "update-ref",
                &request.branch_ref,
                checkpoint_commit,
                &request.expected_old_head,
            ],
            None,
        )
        .map_err(|error| {
            promotion_error(
                PromotionErrorCode::BranchHeadDrift,
                format!(
                    "the branch compare-and-swap for '{}' failed: {}",
                    request.branch_ref, error.message
                ),
            )
        })?;
        self.observe_branch(
            &request.branch_ref,
            &request.expected_old_head,
            checkpoint_commit,
        )
    }

    /// Compensates a branch back from the checkpoint to the expected old head.
    ///
    /// Only ever used for the narrow case the plan allows: the CAS succeeded but
    /// a later step failed. A third value is never overwritten.
    pub fn rollback_branch(
        &self,
        request: &PromotionCheckpointRequest,
        checkpoint_commit: &str,
    ) -> Result<bool, PromotionError> {
        let observed = self.observe_branch(
            &request.branch_ref,
            &request.expected_old_head,
            checkpoint_commit,
        )?;
        match observed {
            BranchObservation::AtOld => Ok(true),
            BranchObservation::AtCheckpoint => {
                git_ok(
                    &self.base_repo,
                    &[
                        "update-ref",
                        &request.branch_ref,
                        &request.expected_old_head,
                        checkpoint_commit,
                    ],
                    None,
                )
                .map_err(|error| {
                    promotion_error(
                        PromotionErrorCode::BranchHeadDrift,
                        format!(
                            "the rollback compare-and-swap for '{}' failed: {}",
                            request.branch_ref, error.message
                        ),
                    )
                })?;
                Ok(true)
            }
            BranchObservation::ThirdValue => Ok(false),
        }
    }

    /// Removes the promotion worktree. The checkpoint ref and commit are
    /// deliberately retained, so a failed attempt keeps its evidence.
    pub fn cleanup_workspace(&self, proof: &CheckpointProof) -> Result<(), PromotionError> {
        if !proof.workspace_root.exists() {
            return Ok(());
        }
        let workspace_arg = proof.workspace_root.to_string_lossy().to_string();
        run_git(
            &self.base_repo,
            &["worktree", "remove", "--force", &workspace_arg],
            None,
        )?;
        Ok(())
    }

    /// Removes every *registered* worktree whose directory name starts with
    /// `name_prefix` (e.g. `promo-<id>-old-g`). Used by the canary runner to
    /// reclaim the arm worktrees a killed attempt left behind, across
    /// generations.
    ///
    /// Ownership is proven by construction: the prefix is derived from the
    /// backend-minted promotion id, and only worktrees inside this owner's own
    /// worktrees root are considered. The checkpoint ref and commit are never
    /// touched.
    pub fn remove_stale_worktrees(&self, name_prefix: &str) -> Result<usize, PromotionError> {
        let listing = git_ok(&self.base_repo, &["worktree", "list", "--porcelain"], None)?;
        let body = String::from_utf8_lossy(&listing.stdout).to_string();
        let mut removed = 0usize;
        for line in body.lines() {
            let Some(path) = line.strip_prefix("worktree ") else {
                continue;
            };
            let path = path.trim();
            let under_root = std::path::Path::new(path).starts_with(&self.worktrees_root);
            let file_name = std::path::Path::new(path)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            if under_root && file_name.starts_with(name_prefix) {
                run_git(
                    &self.base_repo,
                    &["worktree", "remove", "--force", path],
                    None,
                )?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn require_branch_not_checked_out(&self, branch_ref: &str) -> Result<(), PromotionError> {
        let listing = git_ok(&self.base_repo, &["worktree", "list", "--porcelain"], None)?;
        let listing = String::from_utf8_lossy(&listing.stdout).to_string();
        for line in listing.lines() {
            if let Some(value) = line.strip_prefix("branch ") {
                if value.trim() == branch_ref {
                    return Err(promotion_error(
                        PromotionErrorCode::BranchCheckedOut,
                        format!("branch '{branch_ref}' is checked out in another worktree"),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Reads one commit-message trailer value.
fn trailer(body: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    body.lines().find_map(|line| {
        line.strip_prefix(&prefix)
            .map(|value| value.trim().to_string())
    })
}

/// Builds the hardened child environment. The user's global and system Git
/// configuration is neutralised so a hook, alias or URL rewrite can never alter
/// what this owner does, and interactive credential prompts are impossible.
fn git_command(cwd: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(cwd)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("GIT_SSH_COMMAND", "false")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// Refuses an argv that is not on the allowlist or that carries a forbidden
/// token. This runs before any process is spawned, so a forbidden command can
/// never even start.
fn assert_argv_allowed(args: &[&str]) -> Result<(), PromotionError> {
    for arg in args {
        let lowered = arg.to_ascii_lowercase();
        if FORBIDDEN_TOKENS.iter().any(|token| lowered == *token) {
            return Err(promotion_error(
                PromotionErrorCode::RepositoryUnavailable,
                format!("git argument '{arg}' is not permitted for a promotion effect"),
            ));
        }
    }
    // The subcommand is the first positional argument, after any `-c <key=value>`
    // override pair and any other leading option.
    let mut index = 0;
    while index < args.len() {
        let arg = args[index];
        if arg == "-c" {
            index += 2;
            continue;
        }
        if arg.starts_with('-') {
            index += 1;
            continue;
        }
        return if ALLOWED_SUBCOMMANDS.contains(&arg) {
            Ok(())
        } else {
            Err(promotion_error(
                PromotionErrorCode::RepositoryUnavailable,
                format!("git subcommand '{arg}' is not permitted for a promotion effect"),
            ))
        };
    }
    Err(promotion_error(
        PromotionErrorCode::RepositoryUnavailable,
        "a git invocation must name a subcommand",
    ))
}

/// Runs one allowlisted `git` invocation with explicit argv and returns the
/// captured output. A non-zero exit is *not* an error here, so observation can
/// branch on it.
fn run_git(
    cwd: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
) -> Result<std::process::Output, PromotionError> {
    assert_argv_allowed(args)?;
    let mut command = git_command(cwd);
    command.args(args);
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    let mut child = command.spawn().map_err(|error| {
        promotion_error(
            PromotionErrorCode::RepositoryUnavailable,
            format!("failed to run git: {error}"),
        )
    })?;
    if let Some(bytes) = stdin {
        use std::io::Write;
        let mut handle = child.stdin.take().ok_or_else(|| {
            promotion_error(
                PromotionErrorCode::RepositoryUnavailable,
                "git stdin is unavailable",
            )
        })?;
        handle.write_all(bytes).map_err(|error| {
            promotion_error(
                PromotionErrorCode::RepositoryUnavailable,
                format!("failed to pipe into git: {error}"),
            )
        })?;
        drop(handle);
    }
    child.wait_with_output().map_err(|error| {
        promotion_error(
            PromotionErrorCode::RepositoryUnavailable,
            format!("failed to wait for git: {error}"),
        )
    })
}

/// Runs one allowlisted `git` invocation and requires success. The stderr is
/// truncated into the message, so a diagnosis stays bounded.
fn git_ok(
    cwd: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
) -> Result<std::process::Output, PromotionError> {
    let output = run_git(cwd, args, stdin)?;
    if output.status.success() {
        return Ok(output);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stderr: String = stderr.chars().take(400).collect();
    Err(promotion_error(
        PromotionErrorCode::RepositoryUnavailable,
        format!("git {} failed: {stderr}", args.join(" ")),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const NAME: &str = "ChatSpeed Promotion";
    const EMAIL: &str = "promotion@chatspeed.local";

    /// Fixture setup runs raw git: it is the test's own repository bootstrap,
    /// not a promotion effect, so the production argv allowlist does not apply.
    fn fixture_git(cwd: &Path, args: &[&str]) -> std::process::Output {
        let output = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", NAME)
            .env("GIT_AUTHOR_EMAIL", EMAIL)
            .env("GIT_COMMITTER_NAME", NAME)
            .env("GIT_COMMITTER_EMAIL", EMAIL)
            .output()
            .expect("run fixture git");
        assert!(
            output.status.success(),
            "fixture git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    /// Initialises a real temporary repository with one commit on
    /// `refs/heads/experiment/2i` and returns `(repo, old_head, tempdir)`.
    fn repository() -> (PathBuf, String, tempfile::TempDir) {
        let directory = tempdir().expect("tempdir");
        let repo = directory.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir");
        fixture_git(&repo, &["init", "--quiet", "-b", "main"]);
        std::fs::write(repo.join("README.md"), "initial\n").expect("write");
        fixture_git(&repo, &["add", "-A"]);
        fixture_git(
            &repo,
            &["commit", "--quiet", "--no-verify", "-m", "initial"],
        );
        fixture_git(&repo, &["branch", "experiment/2i"]);
        let head = fixture_git(&repo, &["rev-parse", "HEAD"]);
        let head = String::from_utf8_lossy(&head.stdout).trim().to_string();
        (repo, head, directory)
    }

    /// The patch the candidate produced: a plain unified diff that adds one
    /// file, so it applies against any repository with that file absent.
    fn patch_bytes() -> Vec<u8> {
        b"--- /dev/null\n+++ b/improvement.txt\n@@ -0,0 +1 @@\n+better\n".to_vec()
    }

    fn request(promotion_id: &str, old_head: &str, patch: &[u8]) -> PromotionCheckpointRequest {
        PromotionCheckpointRequest {
            promotion_id: promotion_id.to_string(),
            fence: PromotionFence::new("promotion-owner", 1),
            branch_ref: "refs/heads/experiment/2i".to_string(),
            expected_old_head: old_head.to_string(),
            base_revision: "refs/heads/experiment/2i".to_string(),
            git_identity_name: NAME.to_string(),
            git_identity_email: EMAIL.to_string(),
            target_ref: "local-dev".to_string(),
            evidence_hash: "e".repeat(64),
            patch_sha256: digest_hex(patch),
        }
    }

    fn promotion_id() -> String {
        "promo-0123456789abcdef0123456789abcdef".to_string()
    }

    #[test]
    fn a_checkpoint_commit_and_ref_are_created_without_touching_the_base() {
        let (repo, old_head, directory) = repository();
        let worktrees = directory.path().join("worktrees");
        let patch = patch_bytes();
        let owner = PromotionCheckpointOwner::new(&repo, &worktrees);
        let request = request(&promotion_id(), &old_head, &patch);

        let base_status_before = git_ok(&repo, &["status", "--porcelain"], None)
            .expect("status")
            .stdout;
        let base_head_before = git_ok(&repo, &["rev-parse", "HEAD"], None)
            .expect("head")
            .stdout;

        let proof = owner
            .create_checkpoint(&request, &patch)
            .expect("checkpoint created");
        assert_eq!(proof.checkpoint_ref, checkpoint_ref_for(&promotion_id()));
        assert!(proof.checkpoint_commit.len() >= 40);

        // The commit carries the English subject and every evidence trailer.
        let body = git_ok(
            &repo,
            &["log", "-1", "--format=%B", &proof.checkpoint_ref],
            None,
        )
        .expect("log");
        let body = String::from_utf8_lossy(&body.stdout).to_string();
        assert!(body.starts_with(&format!(
            "experiment(promotion): checkpoint {}",
            promotion_id()
        )));
        assert_eq!(
            trailer(&body, "Promotion-Id").as_deref(),
            Some(promotion_id().as_str())
        );
        assert_eq!(
            trailer(&body, "Evidence-Hash").as_deref(),
            Some(&*"e".repeat(64))
        );
        assert_eq!(
            trailer(&body, "Patch-Sha256").as_deref(),
            Some(digest_hex(&patch).as_str())
        );
        assert_eq!(
            trailer(&body, "Base-Revision").as_deref(),
            Some("refs/heads/experiment/2i")
        );
        assert_eq!(trailer(&body, "Target-Ref").as_deref(), Some("local-dev"));

        // The base repository's HEAD and index are untouched, and the branch
        // still points at the old head.
        assert_eq!(
            git_ok(&repo, &["rev-parse", "HEAD"], None)
                .expect("head")
                .stdout,
            base_head_before
        );
        assert_eq!(
            git_ok(&repo, &["status", "--porcelain"], None)
                .expect("status")
                .stdout,
            base_status_before
        );
        assert_eq!(
            owner
                .observe_branch_head("refs/heads/experiment/2i")
                .expect("branch"),
            old_head
        );
        // The patch's effect only exists in the checkpoint commit.
        assert!(!repo.join("improvement.txt").exists());

        // The checkpoint is observed as consistent with this attempt.
        assert_eq!(
            owner.observe_checkpoint(&request).expect("observe"),
            CheckpointObservation::PresentConsistent
        );
        // Re-creating it adopts the existing commit instead of duplicating it.
        let adopted = owner.create_checkpoint(&request, &patch).expect("adopt");
        assert_eq!(adopted.checkpoint_commit, proof.checkpoint_commit);

        // A foreign trailer makes the ref inconsistent, never adoptable.
        let mut tampered = request.clone();
        tampered.evidence_hash = "f".repeat(64);
        assert_eq!(
            owner.observe_checkpoint(&tampered).expect("observe"),
            CheckpointObservation::PresentInconsistent
        );

        // Cleanup removes the worktree but keeps the checkpoint.
        owner.cleanup_workspace(&proof).expect("cleanup");
        assert!(!proof.workspace_root.exists());
        assert!(owner
            .observe_checkpoint(&request)
            .expect("observe")
            .eq(&CheckpointObservation::PresentConsistent));
    }

    #[test]
    fn the_branch_advances_only_by_expected_old_cas() {
        let (repo, old_head, directory) = repository();
        let worktrees = directory.path().join("worktrees");
        let patch = patch_bytes();
        let owner = PromotionCheckpointOwner::new(&repo, &worktrees);
        let request = request(&promotion_id(), &old_head, &patch);
        let proof = owner
            .create_checkpoint(&request, &patch)
            .expect("checkpoint");

        // The branch is still on the old head before the CAS.
        assert_eq!(
            owner
                .observe_branch(
                    "refs/heads/experiment/2i",
                    &old_head,
                    &proof.checkpoint_commit
                )
                .expect("observe"),
            BranchObservation::AtOld
        );
        let observed = owner
            .advance_branch(&request, &proof.checkpoint_commit)
            .expect("advance");
        assert_eq!(observed, BranchObservation::AtCheckpoint);
        assert_eq!(
            owner
                .observe_branch_head("refs/heads/experiment/2i")
                .expect("head"),
            proof.checkpoint_commit
        );
        // The CAS is idempotent for our own effect.
        assert_eq!(
            owner
                .advance_branch(&request, &proof.checkpoint_commit)
                .expect("advance again"),
            BranchObservation::AtCheckpoint
        );

        // A branch moved to a third value (neither the expected old head nor the
        // checkpoint) is refused and never forced.
        fixture_git(
            &repo,
            &[
                "commit",
                "--quiet",
                "--no-verify",
                "--allow-empty",
                "-m",
                "external",
            ],
        );
        let external = String::from_utf8_lossy(&fixture_git(&repo, &["rev-parse", "HEAD"]).stdout)
            .trim()
            .to_string();
        fixture_git(
            &repo,
            &["update-ref", "refs/heads/experiment/2i", &external],
        );
        assert_eq!(
            owner
                .advance_branch(&request, &proof.checkpoint_commit)
                .expect_err("third value")
                .code,
            PromotionErrorCode::BranchHeadDrift
        );
        assert_eq!(
            owner
                .observe_branch_head("refs/heads/experiment/2i")
                .expect("head"),
            external,
            "a refused CAS must not move the branch"
        );
        // Rollback refuses a third value too, and never overwrites it.
        assert!(!owner
            .rollback_branch(&request, &proof.checkpoint_commit)
            .expect("no overwrite"));
        assert_eq!(
            owner
                .observe_branch_head("refs/heads/experiment/2i")
                .expect("head"),
            external
        );

        // With the branch back on our own checkpoint, the checkpoint -> old CAS
        // compensates the advance.
        fixture_git(
            &repo,
            &[
                "update-ref",
                "refs/heads/experiment/2i",
                &proof.checkpoint_commit,
            ],
        );
        assert!(owner
            .rollback_branch(&request, &proof.checkpoint_commit)
            .expect("rollback"));
        assert_eq!(
            owner
                .observe_branch_head("refs/heads/experiment/2i")
                .expect("head"),
            old_head
        );
    }

    #[test]
    fn a_drifted_head_an_unsafe_ref_and_a_bad_patch_fail_closed() {
        let (repo, old_head, directory) = repository();
        let worktrees = directory.path().join("worktrees");
        let patch = patch_bytes();
        let owner = PromotionCheckpointOwner::new(&repo, &worktrees);

        // A drifted expected head is refused before any worktree is created.
        let mut drifted = request(&promotion_id(), &old_head, &patch);
        drifted.expected_old_head = "2".repeat(40);
        assert_eq!(
            owner
                .create_checkpoint(&drifted, &patch)
                .expect_err("drift")
                .code,
            PromotionErrorCode::BranchHeadDrift
        );

        // A relative or non-branch ref is refused.
        let mut unsafe_ref = request(&promotion_id(), &old_head, &patch);
        unsafe_ref.branch_ref = "experiment/2i".to_string();
        assert_eq!(
            owner
                .create_checkpoint(&unsafe_ref, &patch)
                .expect_err("ref")
                .code,
            PromotionErrorCode::UnsafeTargetRef
        );
        let mut remote_ref = request(&promotion_id(), &old_head, &patch);
        remote_ref.branch_ref = "refs/remotes/origin/main".to_string();
        assert_eq!(
            owner
                .create_checkpoint(&remote_ref, &patch)
                .expect_err("remote ref")
                .code,
            PromotionErrorCode::UnsafeTargetRef
        );

        // A patch whose bytes do not match the bound digest is refused.
        let mut tampered = request(&promotion_id(), &old_head, &patch);
        tampered.patch_sha256 = "a".repeat(64);
        assert_eq!(
            owner
                .create_checkpoint(&tampered, &patch)
                .expect_err("digest")
                .code,
            PromotionErrorCode::PatchApplyFailed
        );
        // ...as is a patch that cannot apply at all.
        let bogus = b"--- a/nope\n+++ b/nope\n@@ -1 +1 @@\n-x\n+y\n".to_vec();
        let mut unappliable = request("promo-ffffffffffffffffffffffffffffffff", &old_head, &bogus);
        unappliable.patch_sha256 = digest_hex(&bogus);
        assert_eq!(
            owner
                .create_checkpoint(&unappliable, &bogus)
                .expect_err("apply")
                .code,
            PromotionErrorCode::PatchApplyFailed
        );

        // An invalid promotion id is refused, so a checkpoint ref can never be
        // minted from caller input.
        let mut bad_id = request("not-a-promotion", &old_head, &patch);
        bad_id.promotion_id = "not-a-promotion".to_string();
        assert_eq!(
            owner
                .create_checkpoint(&bad_id, &patch)
                .expect_err("id")
                .code,
            PromotionErrorCode::UnknownPromotion
        );

        // A missing repository fails closed.
        let missing =
            PromotionCheckpointOwner::new(directory.path().join("does-not-exist"), &worktrees);
        assert_eq!(
            missing
                .create_checkpoint(&request(&promotion_id(), &old_head, &patch), &patch)
                .expect_err("missing repo")
                .code,
            PromotionErrorCode::RepositoryUnavailable
        );
    }

    #[test]
    fn remote_and_history_rewriting_commands_are_refused_before_spawn() {
        for forbidden in [
            vec!["push", "origin", "main"],
            vec!["fetch", "origin"],
            vec!["pull"],
            vec!["remote", "add", "origin", "https://example.invalid/repo"],
            vec!["merge", "main"],
            vec!["rebase", "main"],
            vec!["cherry-pick", "abc123"],
            vec!["reset", "--hard"],
            vec!["clone", "https://example.invalid/repo"],
        ] {
            assert_eq!(
                assert_argv_allowed(&forbidden).expect_err("forbidden").code,
                PromotionErrorCode::RepositoryUnavailable,
                "argv {forbidden:?} must be refused"
            );
        }
        // The allowed set still works.
        assert!(assert_argv_allowed(&["rev-parse", "--git-dir"]).is_ok());
        assert!(assert_argv_allowed(&["update-ref", "refs/heads/x", "abc", ""]).is_ok());
        assert!(assert_argv_allowed(&["worktree", "remove", "--force", "/tmp/x"]).is_ok());
        assert_eq!(
            assert_argv_allowed(&["--version"])
                .expect_err("no subcommand")
                .code,
            PromotionErrorCode::RepositoryUnavailable
        );
    }

    #[test]
    fn a_branch_checked_out_elsewhere_is_refused() {
        let (repo, old_head, directory) = repository();
        let worktrees = directory.path().join("worktrees");
        // A second worktree checks out the experiment branch.
        std::fs::create_dir_all(&worktrees).expect("mkdir");
        let other = worktrees.join("user-checkout");
        let other_arg = other.to_string_lossy().to_string();
        git_ok(
            &repo,
            &["worktree", "add", &other_arg, "experiment/2i"],
            None,
        )
        .expect("checkout branch elsewhere");

        let patch = patch_bytes();
        let owner = PromotionCheckpointOwner::new(&repo, &worktrees);
        let request = request(&promotion_id(), &old_head, &patch);
        assert_eq!(
            owner
                .create_checkpoint(&request, &patch)
                .expect_err("checked out")
                .code,
            PromotionErrorCode::BranchCheckedOut
        );
    }
}
