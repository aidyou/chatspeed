//! Run-scoped capability bundle staging and verification (Phase 2G/2H).
//!
//! A bundle is the only way a run gains extra tools: an MCP server or a skill
//! that lives in a *checked-in, allowlisted* directory outside the run. The
//! saga is deliberately split into stages, and only the last stage may be
//! observed by the workflow:
//!
//! ```text
//! acquire (allowlist lookup + strict manifest parse)
//!   -> stage   (copy every declared file, reject symlinks/escapes)
//!   -> verify  (per-file digest + recomputed content digest)
//!   -> lease   (a PreparedCapabilityLease the session may register)
//!   -> release (idempotent removal of the staged tree)
//! ```
//!
//! Contract rules (AC-4, INV-3/INV-4/INV-6/INV-8):
//!
//! - Acquisition only ever reads inside the server-configured allowlisted root;
//!   a `bundle_ref` can never name a path outside it.
//! - Nothing is registered before verification. The lease is minted *after* the
//!   staged bytes match the manifest, so an unverified bundle simply has no
//!   lease to register.
//! - Nothing is written to the global user database or home directories: the
//!   staged tree lives under the experiment domain, scoped by job.
//! - A bundle is staged once per job: a second stage attempt for the same
//!   `(job, bundle)` fails closed instead of silently reusing stale bytes.

use crate::workflow::react::experiment_owner::owner_error;
use crate::workflow::react::experiment_owner::patch::digest_hex;
use crate::workflow::react::experiment_schedule::types::{
    is_valid_key, BundleManifestV1, ScheduleError, ScheduleErrorCode, BUNDLE_MANIFEST_V1,
};
use std::path::{Path, PathBuf};

/// Fixed file name of a bundle manifest inside its allowlisted directory.
pub const BUNDLE_MANIFEST_FILE_NAME: &str = "bundle-manifest.json";

/// A bundle resolved from the allowlisted root, before any copying.
#[derive(Debug, Clone)]
pub struct BundleSource {
    pub bundle_ref: String,
    pub source_root: PathBuf,
    pub manifest: BundleManifestV1,
}

/// A staged, verified bundle.
#[derive(Debug, Clone)]
pub struct StagedBundle {
    /// Stable identity of this installation, derived from job + bundle + digest.
    pub install_id: String,
    pub job_id: String,
    pub bundle_ref: String,
    pub bundle_version: String,
    pub content_digest: String,
    pub staged_root: PathBuf,
    pub manifest: BundleManifestV1,
    /// Number of files whose bytes were verified against the manifest.
    pub verified_files: usize,
}

/// The server-configured allowlist of staged-capable bundles.
#[derive(Debug, Clone)]
pub struct BundleRegistry {
    allowlisted_root: PathBuf,
}

impl BundleRegistry {
    /// Binds the registry to the checked-in bundle root.
    pub fn new(allowlisted_root: impl Into<PathBuf>) -> Self {
        Self {
            allowlisted_root: allowlisted_root.into(),
        }
    }

    pub fn allowlisted_root(&self) -> &Path {
        &self.allowlisted_root
    }

    /// Lists the allowlisted bundle references that carry a manifest.
    pub fn list(&self) -> Result<Vec<String>, ScheduleError> {
        if !self.allowlisted_root.is_dir() {
            return Ok(Vec::new());
        }
        let entries = std::fs::read_dir(&self.allowlisted_root).map_err(|error| {
            owner_error(
                ScheduleErrorCode::BundleRefUnknown,
                format!(
                    "failed to read the bundle root '{}': {error}",
                    self.allowlisted_root.display()
                ),
            )
        })?;
        let mut refs = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                owner_error(
                    ScheduleErrorCode::BundleRefUnknown,
                    format!("failed to read a bundle entry: {error}"),
                )
            })?;
            let path = entry.path();
            if path.is_dir() && path.join(BUNDLE_MANIFEST_FILE_NAME).is_file() {
                if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                    refs.push(name.to_string());
                }
            }
        }
        refs.sort();
        Ok(refs)
    }

    /// Resolves one allowlisted bundle and parses its strict manifest.
    pub fn acquire(&self, bundle_ref: &str) -> Result<BundleSource, ScheduleError> {
        if !is_valid_key(bundle_ref) {
            return Err(owner_error(
                ScheduleErrorCode::BundleRefUnknown,
                format!("'{bundle_ref}' is not a valid bundle reference"),
            ));
        }
        let source_root = self.allowlisted_root.join(bundle_ref);
        if !source_root.is_dir() {
            return Err(owner_error(
                ScheduleErrorCode::BundleRefUnknown,
                format!("no allowlisted bundle '{bundle_ref}' is registered on this host"),
            ));
        }
        let manifest_path = source_root.join(BUNDLE_MANIFEST_FILE_NAME);
        let body = std::fs::read(&manifest_path).map_err(|error| {
            owner_error(
                ScheduleErrorCode::BundleRefUnknown,
                format!("bundle '{bundle_ref}' has no readable manifest: {error}"),
            )
        })?;
        let manifest: BundleManifestV1 = serde_json::from_slice(&body).map_err(|error| {
            owner_error(
                ScheduleErrorCode::BundleManifestInvalid,
                format!("bundle '{bundle_ref}' manifest is not a strict document: {error}"),
            )
        })?;
        if manifest.bundle_ref != bundle_ref {
            return Err(owner_error(
                ScheduleErrorCode::BundleManifestInvalid,
                format!(
                    "bundle directory '{bundle_ref}' declares bundle_ref '{}'",
                    manifest.bundle_ref
                ),
            ));
        }
        if manifest.schema_version != BUNDLE_MANIFEST_V1 {
            return Err(owner_error(
                ScheduleErrorCode::BundleManifestInvalid,
                format!(
                    "bundle '{bundle_ref}' declares schema_version '{}'",
                    manifest.schema_version
                ),
            ));
        }
        manifest.validate()?;
        Ok(BundleSource {
            bundle_ref: bundle_ref.to_string(),
            source_root,
            manifest,
        })
    }
}

/// Stages and verifies one acquired bundle under `<destination_root>/<job_id>`.
pub fn stage_bundle(
    source: &BundleSource,
    destination_root: &Path,
    job_id: &str,
) -> Result<StagedBundle, ScheduleError> {
    if !is_valid_key(job_id) {
        return Err(owner_error(
            ScheduleErrorCode::BundlePathUnsafe,
            "the job id is not usable as a directory name",
        ));
    }
    let job_root = destination_root.join(job_id);
    let destination = job_root.join(&source.bundle_ref);
    if destination.exists() {
        return Err(owner_error(
            ScheduleErrorCode::BundleNotVerifiable,
            format!(
                "bundle '{}' is already staged for job '{job_id}'",
                source.bundle_ref
            ),
        ));
    }
    let staging = job_root.join(format!(".staging-{}", source.bundle_ref));

    let staged = (|| -> Result<StagedBundle, ScheduleError> {
        std::fs::create_dir_all(&staging).map_err(|error| {
            owner_error(
                ScheduleErrorCode::BundleNotVerifiable,
                format!("failed to create '{}': {error}", staging.display()),
            )
        })?;

        let mut verified_files = 0usize;
        for file in &source.manifest.files {
            let from = source.source_root.join(&file.relative_path);
            let to = staging.join(&file.relative_path);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    owner_error(
                        ScheduleErrorCode::BundlePathUnsafe,
                        format!("failed to create '{}': {error}", parent.display()),
                    )
                })?;
            }
            match file.symlink_target.as_deref() {
                // A declared symlink is recreated from its manifest entry and is
                // never followed, so a bundle cannot smuggle a host path in.
                Some(target) => {
                    create_declared_symlink(&to, target)?;
                }
                None => {
                    let bytes = std::fs::read(&from).map_err(|error| {
                        owner_error(
                            ScheduleErrorCode::BundleDigestMismatch,
                            format!(
                                "bundle '{}' is missing '{}': {error}",
                                source.bundle_ref, file.relative_path
                            ),
                        )
                    })?;
                    if bytes.len() as u64 != file.size_bytes || digest_hex(&bytes) != file.sha256 {
                        return Err(owner_error(
                            ScheduleErrorCode::BundleDigestMismatch,
                            format!(
                                "bundle '{}' file '{}' does not match its declared size/digest",
                                source.bundle_ref, file.relative_path
                            ),
                        ));
                    }
                    std::fs::write(&to, &bytes).map_err(|error| {
                        owner_error(
                            ScheduleErrorCode::BundleNotVerifiable,
                            format!("failed to write '{}': {error}", to.display()),
                        )
                    })?;
                    apply_file_mode(&to, file.executable, file.mode)?;
                }
            }
            verified_files += 1;
        }

        // The staged tree carries its own manifest, so a lease can re-verify the
        // installation it is about to register instead of trusting the caller.
        let manifest_bytes = serde_json::to_vec_pretty(&source.manifest).map_err(|error| {
            owner_error(
                ScheduleErrorCode::BundleNotVerifiable,
                format!("the bundle manifest is not serializable: {error}"),
            )
        })?;
        std::fs::write(staging.join(BUNDLE_MANIFEST_FILE_NAME), &manifest_bytes).map_err(
            |error| {
                owner_error(
                    ScheduleErrorCode::BundleNotVerifiable,
                    format!("failed to stage the bundle manifest: {error}"),
                )
            },
        )?;

        // The whole-tree digest must match too: it is what the lease and the
        // campaign fixture bind to.
        let recomputed = source.manifest.computed_content_digest();
        if recomputed != source.manifest.content_digest {
            return Err(owner_error(
                ScheduleErrorCode::BundleDigestMismatch,
                format!(
                    "bundle '{}' content digest does not match its manifest",
                    source.bundle_ref
                ),
            ));
        }

        std::fs::rename(&staging, &destination).map_err(|error| {
            owner_error(
                ScheduleErrorCode::BundleNotVerifiable,
                format!(
                    "failed to publish '{}' as '{}': {error}",
                    staging.display(),
                    destination.display()
                ),
            )
        })?;

        Ok(StagedBundle {
            install_id: install_id_for(job_id, &source.bundle_ref, &source.manifest.content_digest),
            job_id: job_id.to_string(),
            bundle_ref: source.bundle_ref.clone(),
            bundle_version: source.manifest.bundle_version.clone(),
            content_digest: source.manifest.content_digest.clone(),
            staged_root: destination,
            manifest: source.manifest.clone(),
            verified_files,
        })
    })();

    if staged.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    staged
}

/// Removes a staged bundle. Idempotent, and only ever inside the job directory.
pub fn release_staged_bundle(bundle: &StagedBundle) -> Result<(), ScheduleError> {
    if !bundle.staged_root.exists() {
        return Ok(());
    }
    let job_root = bundle.staged_root.parent().ok_or_else(|| {
        owner_error(
            ScheduleErrorCode::BundlePathUnsafe,
            "the staged bundle has no job directory",
        )
    })?;
    if job_root.file_name().and_then(|name| name.to_str()) != Some(bundle.job_id.as_str()) {
        return Err(owner_error(
            ScheduleErrorCode::OwnershipMismatch,
            format!(
                "refusing to remove '{}': it is not inside the job directory of '{}'",
                bundle.staged_root.display(),
                bundle.job_id
            ),
        ));
    }
    std::fs::remove_dir_all(&bundle.staged_root).map_err(|error| {
        owner_error(
            ScheduleErrorCode::BundleNotVerifiable,
            format!(
                "failed to remove '{}': {error}",
                bundle.staged_root.display()
            ),
        )
    })
}

/// Removes everything a job staged, including a staging left behind by a run
/// that outlived its dispatching tick.
///
/// It is idempotent and only ever touches `<destination_root>/<job_id>`, so it
/// can be used by a later tick that no longer holds the original
/// [`StagedBundle`] handles.
pub fn release_job_staging(destination_root: &Path, job_id: &str) -> Result<(), ScheduleError> {
    if !is_valid_key(job_id) {
        return Err(owner_error(
            ScheduleErrorCode::BundlePathUnsafe,
            "the job id is not usable as a directory name",
        ));
    }
    let job_root = destination_root.join(job_id);
    if !job_root.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(&job_root).map_err(|error| {
        owner_error(
            ScheduleErrorCode::BundleNotVerifiable,
            format!("failed to remove '{}': {error}", job_root.display()),
        )
    })
}

/// The stable install identity of a staged bundle.
pub fn install_id_for(job_id: &str, bundle_ref: &str, content_digest: &str) -> String {
    let digest = crate::workflow::react::campaign::domain_hash(
        "cs-bundle:install-id",
        format!("{job_id}\n{bundle_ref}\n{content_digest}").as_bytes(),
    );
    format!("install-{}", &digest[..32])
}

#[cfg(unix)]
fn create_declared_symlink(link: &Path, target: &str) -> Result<(), ScheduleError> {
    std::os::unix::fs::symlink(target, link).map_err(|error| {
        owner_error(
            ScheduleErrorCode::BundlePathUnsafe,
            format!(
                "failed to create the declared symlink '{}': {error}",
                link.display()
            ),
        )
    })
}

#[cfg(not(unix))]
fn create_declared_symlink(_link: &Path, _target: &str) -> Result<(), ScheduleError> {
    Err(owner_error(
        ScheduleErrorCode::BundlePathUnsafe,
        "declared symlinks are not supported on this platform",
    ))
}

#[cfg(unix)]
fn apply_file_mode(path: &Path, executable: bool, mode: u32) -> Result<(), ScheduleError> {
    use std::os::unix::fs::PermissionsExt;
    let effective = if executable && mode & 0o111 == 0 {
        mode | 0o111
    } else {
        mode
    };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(effective)).map_err(|error| {
        owner_error(
            ScheduleErrorCode::BundlePathUnsafe,
            format!("failed to set the mode of '{}': {error}", path.display()),
        )
    })
}

#[cfg(not(unix))]
fn apply_file_mode(_path: &Path, _executable: bool, _mode: u32) -> Result<(), ScheduleError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::experiment_schedule::types::{
        BundleFileV1, BundleMcpServerV1, BundleSkillV1,
    };
    use tempfile::tempdir;

    /// Writes an allowlisted bundle whose manifest matches its bytes.
    fn write_bundle(root: &Path, bundle_ref: &str, script: &str) -> PathBuf {
        let directory = root.join(bundle_ref);
        std::fs::create_dir_all(directory.join("bin")).expect("create bin");
        std::fs::create_dir_all(directory.join("skills")).expect("create skills");
        std::fs::write(directory.join("bin/echo_server.py"), script).expect("write script");
        std::fs::write(directory.join("skills/smoke.md"), "# smoke\n").expect("write skill");

        let mut manifest = BundleManifestV1 {
            schema_version: BUNDLE_MANIFEST_V1.to_string(),
            bundle_ref: bundle_ref.to_string(),
            bundle_version: "1".to_string(),
            content_digest: String::new(),
            files: vec![
                BundleFileV1 {
                    relative_path: "bin/echo_server.py".to_string(),
                    size_bytes: script.len() as u64,
                    sha256: digest_hex(script.as_bytes()),
                    mode: 0o755,
                    executable: true,
                    symlink_target: None,
                },
                BundleFileV1 {
                    relative_path: "skills/smoke.md".to_string(),
                    size_bytes: 8,
                    sha256: digest_hex(b"# smoke\n"),
                    mode: 0o644,
                    executable: false,
                    symlink_target: None,
                },
            ],
            mcp_servers: vec![BundleMcpServerV1 {
                name: "echo".to_string(),
                command: "./bin/echo_server.py".to_string(),
                args: vec![],
                env_secret_refs: vec![],
                env_secret_env_names: vec![],
            }],
            skills: vec![BundleSkillV1 {
                name: "smoke".to_string(),
                entry_path: "skills/smoke.md".to_string(),
            }],
            env_secret_refs: vec![],
        };
        manifest.content_digest = manifest.computed_content_digest();
        std::fs::write(
            directory.join(BUNDLE_MANIFEST_FILE_NAME),
            serde_json::to_vec_pretty(&manifest).expect("serialize"),
        )
        .expect("write manifest");
        directory
    }

    #[test]
    fn an_allowlisted_bundle_is_staged_verified_and_released() {
        let directory = tempdir().expect("tempdir");
        let allowlist = directory.path().join("bundles");
        write_bundle(&allowlist, "smoke-tools", "print('ok')\n");
        let registry = BundleRegistry::new(&allowlist);
        assert_eq!(
            registry.list().expect("list"),
            vec!["smoke-tools".to_string()]
        );

        let source = registry.acquire("smoke-tools").expect("acquire");
        let staged_root = directory.path().join("domain/bundles");
        let staged = stage_bundle(&source, &staged_root, "job-1").expect("stage");
        assert_eq!(staged.verified_files, 2);
        assert_eq!(staged.bundle_version, "1");
        assert!(staged.staged_root.join("bin/echo_server.py").exists());
        assert!(staged.staged_root.join("skills/smoke.md").exists());
        assert_eq!(staged.install_id.len(), "install-".len() + 32);
        assert!(!staged_root.join("job-1/.staging-smoke-tools").exists());

        // The staged bytes match the manifest.
        let staged_script = std::fs::read(staged.staged_root.join("bin/echo_server.py"))
            .expect("read staged script");
        assert_eq!(digest_hex(&staged_script), source.manifest.files[0].sha256);

        // Release is idempotent and removes only the staged tree.
        release_staged_bundle(&staged).expect("release");
        assert!(!staged.staged_root.exists());
        release_staged_bundle(&staged).expect("release again");
        assert!(allowlist.join("smoke-tools").exists());
    }

    #[test]
    fn unknown_and_escaping_bundle_refs_fail_closed() {
        let directory = tempdir().expect("tempdir");
        let registry = BundleRegistry::new(directory.path().join("bundles"));
        let error = registry.acquire("missing").expect_err("unknown bundle");
        assert_eq!(error.code, ScheduleErrorCode::BundleRefUnknown);
        let error = registry.acquire("../escape").expect_err("escaping ref");
        assert_eq!(error.code, ScheduleErrorCode::BundleRefUnknown);
    }

    #[test]
    fn a_tampered_bundle_file_is_not_staged() {
        let directory = tempdir().expect("tempdir");
        let allowlist = directory.path().join("bundles");
        write_bundle(&allowlist, "smoke-tools", "print('ok')\n");
        // Tamper with the script after the manifest was written.
        std::fs::write(
            allowlist.join("smoke-tools/bin/echo_server.py"),
            "print('evil')\n",
        )
        .expect("tamper");

        let registry = BundleRegistry::new(&allowlist);
        let source = registry.acquire("smoke-tools").expect("acquire");
        let staged_root = directory.path().join("domain/bundles");
        let error = stage_bundle(&source, &staged_root, "job-1").expect_err("tampered");
        assert_eq!(error.code, ScheduleErrorCode::BundleDigestMismatch);
        assert!(!staged_root.join("job-1/smoke-tools").exists());
        assert!(!staged_root.join("job-1/.staging-smoke-tools").exists());
    }

    #[test]
    fn a_manifest_with_a_secret_value_is_never_accepted() {
        let directory = tempdir().expect("tempdir");
        let allowlist = directory.path().join("bundles");
        let bundle = write_bundle(&allowlist, "smoke-tools", "print('ok')\n");
        let manifest_path = bundle.join(BUNDLE_MANIFEST_FILE_NAME);
        let mut manifest: BundleManifestV1 =
            serde_json::from_slice(&std::fs::read(&manifest_path).expect("read")).expect("parse");
        manifest.mcp_servers[0].args = vec!["--token=sk-live-abcdef".to_string()];
        manifest.content_digest = manifest.computed_content_digest();
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).expect("serialize"),
        )
        .expect("write");

        let registry = BundleRegistry::new(&allowlist);
        let error = registry.acquire("smoke-tools").expect_err("secret");
        assert_eq!(error.code, ScheduleErrorCode::BundleSecretForbidden);
    }

    #[test]
    fn a_bundle_is_staged_only_once_per_job() {
        let directory = tempdir().expect("tempdir");
        let allowlist = directory.path().join("bundles");
        write_bundle(&allowlist, "smoke-tools", "print('ok')\n");
        let registry = BundleRegistry::new(&allowlist);
        let source = registry.acquire("smoke-tools").expect("acquire");
        let staged_root = directory.path().join("domain/bundles");
        stage_bundle(&source, &staged_root, "job-1").expect("first stage");
        let error = stage_bundle(&source, &staged_root, "job-1").expect_err("second stage");
        assert_eq!(error.code, ScheduleErrorCode::BundleNotVerifiable);
        // A different job stages its own copy.
        let other = stage_bundle(&source, &staged_root, "job-2").expect("other job");
        assert!(other.staged_root.ends_with("job-2/smoke-tools"));
    }

    #[test]
    fn release_refuses_a_bundle_outside_its_job_directory() {
        let directory = tempdir().expect("tempdir");
        let allowlist = directory.path().join("bundles");
        write_bundle(&allowlist, "smoke-tools", "print('ok')\n");
        let registry = BundleRegistry::new(&allowlist);
        let source = registry.acquire("smoke-tools").expect("acquire");
        let staged_root = directory.path().join("domain/bundles");
        let mut staged = stage_bundle(&source, &staged_root, "job-1").expect("stage");
        // A forged record pointing at another job's directory is refused.
        staged.job_id = "job-2".to_string();
        let error = release_staged_bundle(&staged).expect_err("foreign job dir");
        assert_eq!(error.code, ScheduleErrorCode::OwnershipMismatch);
        assert!(staged.staged_root.exists());
    }
}
