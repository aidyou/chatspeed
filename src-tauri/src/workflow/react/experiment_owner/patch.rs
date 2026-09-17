//! Output-patch publication for an owned run workspace (Phase 2G).
//!
//! The collected output of a run is published as an *immutable* pair:
//!
//! ```text
//! <artifacts-root>/<job-id>/patch.diff
//! <artifacts-root>/<job-id>/patch-manifest.json
//! ```
//!
//! Publication is staging-based and atomic (write both files into a staging
//! directory, re-verify their digests from disk, then rename the directory into
//! place), so a reader never observes a half-written artifact and a second
//! publication of the same job fails closed (AC-5).
//!
//! Two safety rules live here because they concern the artifact's *content*:
//!
//! - the patch is scanned for credentials before it is written; a hit refuses
//!   publication (INV-6), and
//! - every path in the patch is checked to be workspace-relative, so a patch
//!   can never describe a write outside the run workspace.

use crate::workflow::react::experiment_owner::owner_error;
use crate::workflow::react::experiment_owner::InputPatch;
use crate::workflow::react::experiment_schedule::types::{ScheduleError, ScheduleErrorCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Fixed schema version of the output-patch manifest.
pub const PATCH_MANIFEST_V1: &str = "run_patch_manifest.v1";
/// Fixed file name of the published diff.
pub const PATCH_FILE_NAME: &str = "patch.diff";
/// Fixed file name of the published manifest.
pub const PATCH_MANIFEST_FILE_NAME: &str = "patch-manifest.json";

/// Everything the manifest binds the patch to.
#[derive(Debug, Clone)]
pub struct PatchContext {
    pub job_id: String,
    pub run_id: Option<String>,
    pub session_id: Option<String>,
    pub candidate_key: String,
    pub base_revision: String,
    /// The artifacts root of the experiment domain.
    pub destination_root: PathBuf,
}

/// One file the patch touches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct PatchFileEntryV1 {
    /// Workspace-relative path.
    pub relative_path: String,
    /// `added`, `modified`, `deleted` or `type_changed`.
    pub change_kind: String,
    pub size_bytes: u64,
    /// sha256 of the file after the change; absent for a deletion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// The immutable manifest that binds one output patch to its run identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct PatchManifestV1 {
    pub schema_version: String,
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub candidate_key: String,
    pub base_revision: String,
    pub diff_sha256: String,
    pub diff_size_bytes: u64,
    pub files: Vec<PatchFileEntryV1>,
    pub created_at_ms: u64,
}

/// A published patch artifact.
#[derive(Debug, Clone)]
pub struct PatchArtifact {
    /// The published directory.
    pub directory: PathBuf,
    /// `patch.diff` relative to the artifacts root, for artifact references.
    pub relative_path: String,
    pub sha256: String,
    pub manifest: PatchManifestV1,
}

/// Lowercase hex sha256 of a byte slice.
pub fn digest_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Credential/secret markers that must never be published inside a patch.
///
/// The list is deliberately small and over-inclusive: a hit refuses
/// publication, so a false positive costs a re-run while a false negative would
/// leak a credential into an artifact.
const SECRET_MARKERS: &[&str] = &[
    "sk-",
    "ghp_",
    "xoxb-",
    "-----BEGIN",
    "AKIA",
    "Bearer eyJ",
    "authorization: bearer",
];

/// Returns the offending marker when the patch text looks like it carries a
/// credential.
pub fn scan_for_secrets(patch_text: &str) -> Option<&'static str> {
    let lowered = patch_text.to_ascii_lowercase();
    SECRET_MARKERS
        .iter()
        .find(|marker| lowered.contains(&marker.to_ascii_lowercase()))
        .copied()
}

/// Rejects a path that is absolute, escaping or otherwise not a plain
/// workspace-relative path.
pub fn is_safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.starts_with('\\')
        && !path.contains('\\')
        && !path.contains('\0')
        && !path.contains(':')
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// Refuses a destination whose ancestor chain contains a symlink.
pub fn reject_symlink_ancestors(path: &Path) -> Result<(), ScheduleError> {
    let mut current = if path.is_absolute() {
        PathBuf::from("/")
    } else {
        PathBuf::new()
    };
    for component in path.components() {
        current.push(component);
        if let Ok(metadata) = std::fs::symlink_metadata(&current) {
            if metadata.file_type().is_symlink() {
                return Err(owner_error(
                    ScheduleErrorCode::ArtifactPublicationFailed,
                    format!(
                        "refusing to publish through the symlinked path '{}'",
                        current.display()
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Publishes the diff and its manifest atomically under `<root>/<job-id>`.
///
/// The manifest is derived from the same bytes that are written, and both files
/// are re-read and re-hashed from disk before the directory is renamed into
/// place, so a published artifact is always self-consistent.
pub fn publish_output_patch(
    diff_bytes: &[u8],
    entries: Vec<PatchFileEntryV1>,
    context: &PatchContext,
    now_ms: u64,
) -> Result<PatchArtifact, ScheduleError> {
    let diff_text = String::from_utf8_lossy(diff_bytes);
    if let Some(marker) = scan_for_secrets(&diff_text) {
        return Err(owner_error(
            ScheduleErrorCode::OutputPatchScanFailed,
            format!("the output patch carries the credential marker '{marker}'"),
        ));
    }
    for entry in &entries {
        if !is_safe_relative_path(&entry.relative_path) {
            return Err(owner_error(
                ScheduleErrorCode::WorkspaceEscape,
                format!(
                    "the output patch references the unsafe path '{}'",
                    entry.relative_path
                ),
            ));
        }
    }

    reject_symlink_ancestors(&context.destination_root)?;
    let destination = context.destination_root.join(&context.job_id);
    if destination.exists() {
        return Err(owner_error(
            ScheduleErrorCode::ArtifactPublicationFailed,
            format!(
                "the patch of job '{}' is already published at '{}'",
                context.job_id,
                destination.display()
            ),
        ));
    }

    let manifest = PatchManifestV1 {
        schema_version: PATCH_MANIFEST_V1.to_string(),
        job_id: context.job_id.clone(),
        run_id: context.run_id.clone(),
        session_id: context.session_id.clone(),
        candidate_key: context.candidate_key.clone(),
        base_revision: context.base_revision.clone(),
        diff_sha256: digest_hex(diff_bytes),
        diff_size_bytes: diff_bytes.len() as u64,
        files: entries,
        created_at_ms: now_ms,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| {
        owner_error(
            ScheduleErrorCode::ArtifactPublicationFailed,
            format!("the patch manifest is not serializable: {error}"),
        )
    })?;

    let staging = staging_dir(&context.destination_root, &context.job_id)?;
    let staged = (|| -> Result<(), ScheduleError> {
        std::fs::create_dir_all(&staging).map_err(|error| {
            owner_error(
                ScheduleErrorCode::ArtifactPublicationFailed,
                format!("failed to create '{}': {error}", staging.display()),
            )
        })?;
        write_and_verify(
            &staging.join(PATCH_FILE_NAME),
            diff_bytes,
            &manifest.diff_sha256,
        )?;
        write_and_verify(
            &staging.join(PATCH_MANIFEST_FILE_NAME),
            &manifest_bytes,
            &digest_hex(&manifest_bytes),
        )?;
        std::fs::rename(&staging, &destination).map_err(|error| {
            owner_error(
                ScheduleErrorCode::ArtifactPublicationFailed,
                format!(
                    "failed to publish '{}' as '{}': {error}",
                    staging.display(),
                    destination.display()
                ),
            )
        })?;
        Ok(())
    })();
    if stored_cleanup_needed(&staged) {
        let _ = std::fs::remove_dir_all(&staging);
    }
    staged?;

    Ok(PatchArtifact {
        directory: destination.clone(),
        relative_path: format!("{}/{}", context.job_id, PATCH_FILE_NAME),
        sha256: manifest.diff_sha256.clone(),
        manifest,
    })
}

/// Whether a failed publication left a staging directory to clean up.
fn stored_cleanup_needed(staged: &Result<(), ScheduleError>) -> bool {
    staged.is_err()
}

fn staging_dir(root: &Path, job_id: &str) -> Result<PathBuf, ScheduleError> {
    if !crate::workflow::react::experiment_schedule::types::is_valid_key(job_id) {
        return Err(owner_error(
            ScheduleErrorCode::ArtifactPublicationFailed,
            "the job id is not usable as a directory name",
        ));
    }
    Ok(root.join(format!(".staging-{job_id}")))
}

fn write_and_verify(path: &Path, bytes: &[u8], expected: &str) -> Result<(), ScheduleError> {
    std::fs::write(path, bytes).map_err(|error| {
        owner_error(
            ScheduleErrorCode::ArtifactPublicationFailed,
            format!("failed to write '{}': {error}", path.display()),
        )
    })?;
    let read_back = std::fs::read(path).map_err(|error| {
        owner_error(
            ScheduleErrorCode::ArtifactPublicationFailed,
            format!("failed to re-read '{}': {error}", path.display()),
        )
    })?;
    let actual = digest_hex(&read_back);
    if actual != expected {
        return Err(owner_error(
            ScheduleErrorCode::ArtifactPublicationFailed,
            format!("'{}' does not match its expected digest", path.display()),
        ));
    }
    Ok(())
}

/// Parses `git status --porcelain -z` output into `(status, path)` pairs.
///
/// A rename/copy entry carries a second NUL-separated path, which is consumed
/// and reported as the entry's target so the manifest always names the file
/// that exists after the change.
pub fn parse_porcelain_z(
    bytes: &[u8],
) -> Result<Vec<(String, String, Option<String>)>, ScheduleError> {
    let fields: Vec<&[u8]> = bytes.split(|byte| *byte == 0).collect();
    let mut entries = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let field = fields[index];
        if field.is_empty() {
            index += 1;
            continue;
        }
        if field.len() < 4 {
            return Err(owner_error(
                ScheduleErrorCode::WorkspaceEscape,
                "the workspace status output is malformed",
            ));
        }
        let status = String::from_utf8_lossy(&field[..2]).to_string();
        let path = String::from_utf8_lossy(&field[3..]).to_string();
        let mut origin = None;
        if status.starts_with('R') || status.starts_with('C') {
            // The next NUL field is the original path of a rename/copy.
            if index + 1 < fields.len() {
                origin = Some(String::from_utf8_lossy(fields[index + 1]).to_string());
                index += 1;
            }
        }
        if !is_safe_relative_path(&path) {
            return Err(owner_error(
                ScheduleErrorCode::WorkspaceEscape,
                format!("the workspace reported an unsafe path '{path}'"),
            ));
        }
        // `git` quotes paths with special characters; a quoted path is not a
        // plain workspace-relative path, so it is rejected rather than
        // mis-parsed.
        if path.starts_with('"') || path.ends_with('"') {
            return Err(owner_error(
                ScheduleErrorCode::WorkspaceEscape,
                format!("the workspace reported an unparsable path {path}"),
            ));
        }
        entries.push((status, path, origin));
        index += 1;
    }
    Ok(entries)
}

/// Maps a porcelain status to the manifest's `change_kind`.
pub fn change_kind_for_status(status: &str) -> &'static str {
    match status {
        "??" => "added",
        "!!" => "modified",
        _ if status.contains('D') => "deleted",
        _ if status.contains('R') => "renamed",
        _ if status.contains('C') => "copied",
        _ if status.contains('T') => "type_changed",
        _ if status.contains('A') => "added",
        _ => "modified",
    }
}

/// Runs one `git` invocation with explicit argv (never a shell) and optional
/// stdin. Shared by every owner that manipulates a Git workspace, so patch
/// handling exists exactly once.
pub fn git_run(cwd: &Path, args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>, ScheduleError> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(cwd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    let mut child = command.spawn().map_err(|error| {
        owner_error(
            ScheduleErrorCode::ExecutorUnavailable,
            format!("failed to run git: {error}"),
        )
    })?;
    if let Some(bytes) = stdin {
        use std::io::Write;
        let mut handle = child.stdin.take().ok_or_else(|| {
            owner_error(
                ScheduleErrorCode::ExecutorUnavailable,
                "git stdin is unavailable",
            )
        })?;
        handle.write_all(bytes).map_err(|error| {
            owner_error(
                ScheduleErrorCode::ExecutorUnavailable,
                format!("failed to pipe into git: {error}"),
            )
        })?;
        drop(handle);
    }
    let output = child.wait_with_output().map_err(|error| {
        owner_error(
            ScheduleErrorCode::ExecutorUnavailable,
            format!("failed to wait for git: {error}"),
        )
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(owner_error(
            ScheduleErrorCode::ExecutorUnavailable,
            format!("git {} failed: {}", args.join(" "), stderr),
        ));
    }
    Ok(output.stdout)
}

/// Applies a digest-bound input patch inside a workspace.
///
/// `git apply --check` runs first, so a patch that does not apply cleanly is
/// rejected before anything is written.
pub fn apply_patch_in_workspace(workspace: &Path, patch: &InputPatch) -> Result<(), ScheduleError> {
    let actual = digest_hex(&patch.bytes);
    if actual != patch.digest {
        return Err(owner_error(
            ScheduleErrorCode::InputPatchRejected,
            "the input patch does not match its declared digest",
        ));
    }
    git_run(workspace, &["apply", "--check", "-"], Some(&patch.bytes)).map_err(|error| {
        owner_error(
            ScheduleErrorCode::InputPatchRejected,
            format!("the input patch does not apply: {}", error.message),
        )
    })?;
    git_run(workspace, &["apply", "-"], Some(&patch.bytes)).map_err(|error| {
        owner_error(
            ScheduleErrorCode::InputPatchRejected,
            format!("the input patch failed to apply: {}", error.message),
        )
    })?;
    Ok(())
}

/// Collects and publishes the workspace's output as an immutable patch.
///
/// The workspace's own index is used (`git add -A` inside the workspace), so the
/// base repository's index and HEAD are never touched.
pub fn collect_workspace_patch(
    workspace: &Path,
    context: &PatchContext,
    now_ms: u64,
) -> Result<PatchArtifact, ScheduleError> {
    let reported = parse_porcelain_z(&git_run(workspace, &["status", "--porcelain", "-z"], None)?)?;

    let mut entries = Vec::with_capacity(reported.len());
    for (status, relative_path, origin) in &reported {
        if !is_safe_relative_path(relative_path) {
            return Err(owner_error(
                ScheduleErrorCode::WorkspaceEscape,
                format!("the run reported the unsafe path '{relative_path}'"),
            ));
        }
        if let Some(origin) = origin {
            if !is_safe_relative_path(origin) {
                return Err(owner_error(
                    ScheduleErrorCode::WorkspaceEscape,
                    format!("the run reported the unsafe origin path '{origin}'"),
                ));
            }
        }
        if let Some(entry) = workspace_file_entry(workspace, status, relative_path)? {
            entries.push(entry);
        }
    }

    git_run(workspace, &["add", "-A"], None)?;
    let diff = git_run(workspace, &["diff", "--cached", "--binary", "HEAD"], None)?;
    publish_output_patch(&diff, entries, context, now_ms)
}

/// Builds one manifest entry for a workspace-relative path.
fn workspace_file_entry(
    workspace: &Path,
    status: &str,
    relative_path: &str,
) -> Result<Option<PatchFileEntryV1>, ScheduleError> {
    let absolute = workspace.join(relative_path);
    let change_kind = change_kind_for_status(status);
    if change_kind == "deleted" {
        return Ok(Some(PatchFileEntryV1 {
            relative_path: relative_path.to_string(),
            change_kind: change_kind.to_string(),
            size_bytes: 0,
            sha256: None,
        }));
    }
    let metadata = std::fs::symlink_metadata(&absolute).map_err(|error| {
        owner_error(
            ScheduleErrorCode::WorkspaceEscape,
            format!("failed to inspect '{}': {error}", absolute.display()),
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(owner_error(
            ScheduleErrorCode::WorkspaceEscape,
            format!("the run created the symlink '{relative_path}', which is not publishable"),
        ));
    }
    if metadata.is_dir() {
        // A submodule/gitlink directory is reported by its own status entry.
        return Ok(None);
    }
    let bytes = std::fs::read(&absolute).map_err(|error| {
        owner_error(
            ScheduleErrorCode::WorkspaceEscape,
            format!("failed to read '{}': {error}", absolute.display()),
        )
    })?;
    Ok(Some(PatchFileEntryV1 {
        relative_path: relative_path.to_string(),
        change_kind: change_kind.to_string(),
        size_bytes: bytes.len() as u64,
        sha256: Some(digest_hex(&bytes)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn context(root: &Path) -> PatchContext {
        PatchContext {
            job_id: "job-1".to_string(),
            run_id: Some("run-1".to_string()),
            session_id: Some("session-1".to_string()),
            candidate_key: "baseline".to_string(),
            base_revision: "refs/heads/main".to_string(),
            destination_root: root.to_path_buf(),
        }
    }

    fn entries() -> Vec<PatchFileEntryV1> {
        vec![PatchFileEntryV1 {
            relative_path: "src/lib.rs".to_string(),
            change_kind: "modified".to_string(),
            size_bytes: 12,
            sha256: Some("b".repeat(64)),
        }]
    }

    #[test]
    fn publication_is_atomic_and_binds_the_run_identity() {
        let directory = tempdir().expect("tempdir");
        let diff = b"--- a/src/lib.rs\n+++ b/src/lib.rs\n";
        let artifact =
            publish_output_patch(diff, entries(), &context(directory.path()), 42).expect("publish");

        assert_eq!(artifact.sha256, digest_hex(diff));
        assert!(artifact.directory.join(PATCH_FILE_NAME).exists());
        assert!(artifact.directory.join(PATCH_MANIFEST_FILE_NAME).exists());
        assert_eq!(artifact.relative_path, "job-1/patch.diff");
        assert_eq!(artifact.manifest.job_id, "job-1");
        assert_eq!(artifact.manifest.run_id.as_deref(), Some("run-1"));
        assert_eq!(artifact.manifest.base_revision, "refs/heads/main");
        assert_eq!(artifact.manifest.created_at_ms, 42);
        // No staging directory is left behind.
        assert!(!directory.path().join(".staging-job-1").exists());
    }

    #[test]
    fn a_second_publication_of_the_same_job_fails_closed() {
        let directory = tempdir().expect("tempdir");
        publish_output_patch(b"x", entries(), &context(directory.path()), 1).expect("first");
        let error = publish_output_patch(b"y", entries(), &context(directory.path()), 2)
            .expect_err("second publication must fail");
        assert_eq!(error.code, ScheduleErrorCode::ArtifactPublicationFailed);
        // The first artifact is untouched.
        let published = std::fs::read(directory.path().join("job-1").join(PATCH_FILE_NAME))
            .expect("published patch");
        assert_eq!(published, b"x");
    }

    #[test]
    fn a_credential_in_the_patch_refuses_publication() {
        let directory = tempdir().expect("tempdir");
        for marker in ["sk-live-abcdef", "Authorization: Bearer eyJhbGciOi"] {
            let diff = format!("+++ b/env\n+API_KEY={marker}\n");
            let error =
                publish_output_patch(diff.as_bytes(), entries(), &context(directory.path()), 1)
                    .expect_err("secret must not be published");
            assert_eq!(error.code, ScheduleErrorCode::OutputPatchScanFailed);
            assert!(!directory.path().join("job-1").exists());
        }
    }

    #[test]
    fn an_escaping_path_refuses_publication() {
        let directory = tempdir().expect("tempdir");
        let bad = vec![PatchFileEntryV1 {
            relative_path: "../outside.txt".to_string(),
            change_kind: "modified".to_string(),
            size_bytes: 1,
            sha256: None,
        }];
        let error = publish_output_patch(b"x", bad, &context(directory.path()), 1)
            .expect_err("escaping path");
        assert_eq!(error.code, ScheduleErrorCode::WorkspaceEscape);
    }

    #[test]
    fn safe_relative_paths_are_classified() {
        assert!(is_safe_relative_path("src/lib.rs"));
        assert!(is_safe_relative_path("a/b/c.txt"));
        for unsafe_path in ["/etc/passwd", "../x", "a/../b", "C:/x", "a\\b", "", "./a"] {
            assert!(
                !is_safe_relative_path(unsafe_path),
                "path {unsafe_path} must be rejected"
            );
        }
    }

    #[test]
    fn porcelain_entries_are_parsed_and_validated() {
        let parsed = parse_porcelain_z(b" M src/lib.rs\0?? notes.txt\0").expect("parse");
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed[0],
            (" M".to_string(), "src/lib.rs".to_string(), None)
        );
        assert_eq!(parsed[1].1, "notes.txt");
        assert_eq!(change_kind_for_status("??"), "added");
        assert_eq!(change_kind_for_status(" M"), "modified");
        assert_eq!(change_kind_for_status(" D"), "deleted");
        assert_eq!(change_kind_for_status("R "), "renamed");
        assert_eq!(change_kind_for_status("T "), "type_changed");

        // A rename consumes its origin field.
        let renamed = parse_porcelain_z(b"R  new.txt\0old.txt\0").expect("parse");
        assert_eq!(renamed.len(), 1);
        assert_eq!(renamed[0].1, "new.txt");
        assert_eq!(renamed[0].2.as_deref(), Some("old.txt"));

        // An escaping path fails closed.
        let error = parse_porcelain_z(b" M ../escape\0").expect_err("escape");
        assert_eq!(error.code, ScheduleErrorCode::WorkspaceEscape);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_destination_ancestor_is_refused() {
        let directory = tempdir().expect("tempdir");
        let real = directory.path().join("real");
        std::fs::create_dir_all(&real).expect("create real");
        let link = directory.path().join("link");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");
        let error = reject_symlink_ancestors(&link.join("job-1")).expect_err("symlink ancestor");
        assert_eq!(error.code, ScheduleErrorCode::ArtifactPublicationFailed);
    }
}
