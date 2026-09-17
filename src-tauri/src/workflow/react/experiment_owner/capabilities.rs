//! Run-scoped capability leases: the only thing a session may register.
//!
//! A [`PreparedCapabilityLease`] is minted **exclusively** from a staged,
//! verified bundle. There is no constructor that accepts an unverified tree, so
//! "register only verified capabilities" is enforced by the type rather than by
//! a convention a caller could forget (AC-4/INV-3).
//!
//! The lease is session scoped: it resolves every declared program and skill to
//! an absolute path *inside the staged bundle*, resolves secret references to
//! values at run time, and carries no global state. Registering it therefore
//! cannot modify the user's global MCP/skill configuration or home directory.
//!
//! Secret values are held only in memory for the lifetime of the lease. They
//! are never written to the experiment database, a journal, an artifact or a
//! log line (INV-6).

use crate::workflow::react::experiment_owner::bundle::{StagedBundle, BUNDLE_MANIFEST_FILE_NAME};
use crate::workflow::react::experiment_owner::owner_error;
use crate::workflow::react::experiment_schedule::types::{ScheduleError, ScheduleErrorCode};
use std::path::{Path, PathBuf};

/// An MCP server a verified bundle contributes to one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredMcpServer {
    pub name: String,
    /// Absolute path of the program inside the staged bundle.
    pub command: PathBuf,
    pub args: Vec<String>,
    pub working_directory: PathBuf,
    /// Environment variables resolved from secret references, in declaration
    /// order. Values never leave this process.
    pub env: Vec<(String, String)>,
}

/// A skill a verified bundle contributes to one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredSkill {
    pub name: String,
    /// Absolute path of the skill's entry document inside the staged bundle.
    pub entry_path: PathBuf,
    pub bundle_root: PathBuf,
}

/// The typed registration a lease carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRegistration {
    pub mcp_servers: Vec<RegisteredMcpServer>,
    pub skills: Vec<RegisteredSkill>,
}

/// The secret values a domain is allowed to resolve for a run.
///
/// It is built from the restricted credential input (a config package or a key
/// file) and is never persisted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SecretEnvironment {
    values: Vec<(String, String)>,
}

impl SecretEnvironment {
    pub fn new(values: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut values: Vec<(String, String)> = values.into_iter().collect();
        values.sort_by(|left, right| left.0.cmp(&right.0));
        Self { values }
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Resolves one declared secret reference. A missing reference fails closed:
    /// a run never silently starts without a capability it declared.
    pub fn resolve(&self, reference: &str) -> Result<String, ScheduleError> {
        self.values
            .iter()
            .find(|(name, _)| name == reference)
            .map(|(_, value)| value.clone())
            .ok_or_else(|| {
                owner_error(
                    ScheduleErrorCode::BundleSecretForbidden,
                    format!("the secret reference '{reference}' is not available in this domain"),
                )
            })
    }
}

/// A verified, run-scoped capability lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCapabilityLease {
    pub install_id: String,
    pub job_id: String,
    pub owner_token_hash: String,
    pub bundle_ref: String,
    pub bundle_version: String,
    pub content_digest: String,
    pub staged_root: PathBuf,
    pub registration: CapabilityRegistration,
}

impl PreparedCapabilityLease {
    /// Mints a lease from a **staged, verified** bundle.
    ///
    /// This is the only constructor. It re-checks that the staged tree still
    /// exists, still carries its manifest, and that every declared program and
    /// skill resolves to a regular file inside that tree — so a bundle that was
    /// deleted or replaced between verification and registration cannot be
    /// registered by accident.
    pub fn from_verified_bundle(
        bundle: &StagedBundle,
        owner_token_hash: &str,
        secrets: &SecretEnvironment,
    ) -> Result<Self, ScheduleError> {
        if !bundle.staged_root.is_dir() {
            return Err(owner_error(
                ScheduleErrorCode::BundleNotVerifiable,
                format!(
                    "the staged bundle '{}' is gone; it cannot be registered",
                    bundle.staged_root.display()
                ),
            ));
        }
        if !bundle.staged_root.join(BUNDLE_MANIFEST_FILE_NAME).is_file() {
            return Err(owner_error(
                ScheduleErrorCode::BundleNotVerifiable,
                format!(
                    "the staged bundle '{}' has no manifest; it cannot be registered",
                    bundle.staged_root.display()
                ),
            ));
        }

        let mut mcp_servers = Vec::with_capacity(bundle.manifest.mcp_servers.len());
        for server in &bundle.manifest.mcp_servers {
            let command = resolve_inside(&bundle.staged_root, &server.command)?;
            if !command.is_file() {
                return Err(owner_error(
                    ScheduleErrorCode::BundleNotVerifiable,
                    format!(
                        "the bundle program '{}' is missing from the staged tree",
                        server.command
                    ),
                ));
            }
            let mut env = Vec::with_capacity(server.env_secret_refs.len());
            for (reference, variable) in server
                .env_secret_refs
                .iter()
                .zip(server.env_secret_env_names.iter())
            {
                env.push((variable.clone(), secrets.resolve(reference)?));
            }
            mcp_servers.push(RegisteredMcpServer {
                name: server.name.clone(),
                command,
                args: server.args.clone(),
                working_directory: bundle.staged_root.clone(),
                env,
            });
        }

        let mut skills = Vec::with_capacity(bundle.manifest.skills.len());
        for skill in &bundle.manifest.skills {
            let entry_path = resolve_inside(&bundle.staged_root, &skill.entry_path)?;
            if !entry_path.is_file() {
                return Err(owner_error(
                    ScheduleErrorCode::BundleNotVerifiable,
                    format!(
                        "the skill entry '{}' is missing from the staged tree",
                        skill.entry_path
                    ),
                ));
            }
            skills.push(RegisteredSkill {
                name: skill.name.clone(),
                entry_path,
                bundle_root: bundle.staged_root.clone(),
            });
        }

        Ok(Self {
            install_id: bundle.install_id.clone(),
            job_id: bundle.job_id.clone(),
            owner_token_hash: owner_token_hash.to_string(),
            bundle_ref: bundle.bundle_ref.clone(),
            bundle_version: bundle.bundle_version.clone(),
            content_digest: bundle.content_digest.clone(),
            staged_root: bundle.staged_root.clone(),
            registration: CapabilityRegistration {
                mcp_servers,
                skills,
            },
        })
    }

    /// Whether this lease belongs to a job's current owner generation.
    pub fn belongs_to(&self, job_id: &str, owner_token_hash: &str) -> bool {
        self.job_id == job_id && self.owner_token_hash == owner_token_hash
    }

    /// A redacted summary for logs: identity only, never a resolved secret.
    pub fn describe(&self) -> String {
        format!(
            "install={} bundle={}@{} digest={} mcp_servers={} skills={}",
            self.install_id,
            self.bundle_ref,
            self.bundle_version,
            &self.content_digest[..12.min(self.content_digest.len())],
            self.registration.mcp_servers.len(),
            self.registration.skills.len()
        )
    }
}

/// Resolves a bundle-relative path to an absolute path inside the staged root.
///
/// The manifest validator has already rejected absolute and escaping paths; this
/// function re-checks, so a lease can never hand the session a path outside the
/// bundle even if a manifest was produced by an older build.
fn resolve_inside(root: &Path, relative: &str) -> Result<PathBuf, ScheduleError> {
    // A declared program is written as `./bin/program`; normalize the leading
    // `./` first so the remainder is a plain relative path.
    let relative = relative.strip_prefix("./").unwrap_or(relative);
    if relative.is_empty()
        || !crate::workflow::react::experiment_owner::patch::is_safe_relative_path(relative)
    {
        return Err(owner_error(
            ScheduleErrorCode::BundlePathUnsafe,
            format!("the bundle path '{relative}' is not workspace-relative"),
        ));
    }
    let resolved = root.join(relative);
    if !resolved.starts_with(root) {
        return Err(owner_error(
            ScheduleErrorCode::BundlePathUnsafe,
            format!("the bundle path '{relative}' escapes the staged tree"),
        ));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::experiment_owner::bundle::{
        release_staged_bundle, stage_bundle, BundleRegistry, BUNDLE_MANIFEST_FILE_NAME as MANIFEST,
    };
    use crate::workflow::react::experiment_owner::patch::digest_hex;
    use crate::workflow::react::experiment_schedule::types::{
        BundleFileV1, BundleManifestV1, BundleMcpServerV1, BundleSkillV1, BUNDLE_MANIFEST_V1,
    };
    use tempfile::tempdir;

    fn write_bundle(root: &Path, bundle_ref: &str, secret_refs: bool) -> PathBuf {
        let directory = root.join(bundle_ref);
        std::fs::create_dir_all(directory.join("bin")).expect("create bin");
        std::fs::create_dir_all(directory.join("skills")).expect("create skills");
        let script = "print('ok')\n";
        std::fs::write(directory.join("bin/echo_server.py"), script).expect("write script");
        std::fs::write(directory.join("skills/smoke.md"), "# smoke\n").expect("write skill");

        let (env_secret_refs, env_secret_env_names) = if secret_refs {
            (
                vec!["smoke-token".to_string()],
                vec!["SMOKE_TOKEN".to_string()],
            )
        } else {
            (vec![], vec![])
        };
        let mut manifest = BundleManifestV1 {
            schema_version: BUNDLE_MANIFEST_V1.to_string(),
            bundle_ref: bundle_ref.to_string(),
            bundle_version: "2".to_string(),
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
                args: vec!["--serve".to_string()],
                env_secret_refs,
                env_secret_env_names,
            }],
            skills: vec![BundleSkillV1 {
                name: "smoke".to_string(),
                entry_path: "skills/smoke.md".to_string(),
            }],
            env_secret_refs: vec![],
        };
        manifest.content_digest = manifest.computed_content_digest();
        std::fs::write(
            directory.join(MANIFEST),
            serde_json::to_vec_pretty(&manifest).expect("serialize"),
        )
        .expect("write manifest");
        directory
    }

    fn staged(directory: &Path, secret_refs: bool, job_id: &str) -> StagedBundle {
        let allowlist = directory.join("bundles");
        write_bundle(&allowlist, "smoke-tools", secret_refs);
        let registry = BundleRegistry::new(&allowlist);
        let source = registry.acquire("smoke-tools").expect("acquire");
        stage_bundle(&source, &directory.join("domain/bundles"), job_id).expect("stage")
    }

    #[test]
    fn a_verified_bundle_yields_a_session_local_lease() {
        let directory = tempdir().expect("tempdir");
        let bundle = staged(directory.path(), false, "job-1");
        let lease = PreparedCapabilityLease::from_verified_bundle(
            &bundle,
            &"a".repeat(64),
            &SecretEnvironment::default(),
        )
        .expect("lease");

        assert_eq!(lease.install_id, bundle.install_id);
        assert_eq!(lease.bundle_version, "2");
        assert!(lease.belongs_to("job-1", &"a".repeat(64)));
        assert!(!lease.belongs_to("job-2", &"a".repeat(64)));
        assert!(!lease.belongs_to("job-1", &"b".repeat(64)));

        // The registration points at absolute paths inside the staged tree.
        let server = lease.registration.mcp_servers.first().expect("server");
        assert_eq!(server.name, "echo");
        assert!(server.command.starts_with(&bundle.staged_root));
        assert!(server.command.ends_with("bin/echo_server.py"));
        assert_eq!(server.args, vec!["--serve".to_string()]);
        assert!(server.env.is_empty());
        let skill = lease.registration.skills.first().expect("skill");
        assert_eq!(skill.name, "smoke");
        assert!(skill.entry_path.starts_with(&bundle.staged_root));

        // The log summary never carries a path or a secret.
        let described = lease.describe();
        assert!(described.contains("smoke-tools"));
        assert!(!described.contains(&bundle.staged_root.to_string_lossy().to_string()));
    }

    #[test]
    fn a_removed_staged_tree_can_never_be_registered() {
        let directory = tempdir().expect("tempdir");
        let bundle = staged(directory.path(), false, "job-1");
        release_staged_bundle(&bundle).expect("release");
        let error = PreparedCapabilityLease::from_verified_bundle(
            &bundle,
            &"a".repeat(64),
            &SecretEnvironment::default(),
        )
        .expect_err("gone");
        assert_eq!(error.code, ScheduleErrorCode::BundleNotVerifiable);
    }

    #[test]
    fn a_missing_declared_program_fails_closed() {
        let directory = tempdir().expect("tempdir");
        let bundle = staged(directory.path(), false, "job-1");
        std::fs::remove_file(bundle.staged_root.join("bin/echo_server.py"))
            .expect("remove program");
        let error = PreparedCapabilityLease::from_verified_bundle(
            &bundle,
            &"a".repeat(64),
            &SecretEnvironment::default(),
        )
        .expect_err("program gone");
        assert_eq!(error.code, ScheduleErrorCode::BundleNotVerifiable);
    }

    #[test]
    fn a_declared_secret_reference_is_resolved_or_refused() {
        let directory = tempdir().expect("tempdir");
        let bundle = staged(directory.path(), true, "job-1");

        let error = PreparedCapabilityLease::from_verified_bundle(
            &bundle,
            &"a".repeat(64),
            &SecretEnvironment::default(),
        )
        .expect_err("missing secret");
        assert_eq!(error.code, ScheduleErrorCode::BundleSecretForbidden);

        let secrets = SecretEnvironment::new([("smoke-token".to_string(), "s3cret".to_string())]);
        let lease =
            PreparedCapabilityLease::from_verified_bundle(&bundle, &"a".repeat(64), &secrets)
                .expect("lease");
        let server = lease.registration.mcp_servers.first().expect("server");
        assert_eq!(
            server.env,
            vec![("SMOKE_TOKEN".to_string(), "s3cret".to_string())]
        );
        // The redacted summary never contains the secret value.
        assert!(!lease.describe().contains("s3cret"));
    }

    #[test]
    fn bundle_paths_never_resolve_outside_the_staged_tree() {
        let directory = tempdir().expect("tempdir");
        let root = directory.path();
        let error = resolve_inside(root, "../escape").expect_err("escape");
        assert_eq!(error.code, ScheduleErrorCode::BundlePathUnsafe);
        let error = resolve_inside(root, "/etc/passwd").expect_err("absolute");
        assert_eq!(error.code, ScheduleErrorCode::BundlePathUnsafe);
        // A declared program keeps its leading `./` form working.
        let resolved = resolve_inside(root, "./bin/program").expect("relative");
        assert!(resolved.starts_with(root));
        assert!(resolved.ends_with("bin/program"));
    }

    #[test]
    fn the_secret_environment_resolves_only_declared_references() {
        let secrets = SecretEnvironment::new([
            ("b-ref".to_string(), "second".to_string()),
            ("a-ref".to_string(), "first".to_string()),
        ]);
        assert_eq!(secrets.resolve("a-ref").expect("a"), "first");
        assert_eq!(secrets.resolve("b-ref").expect("b"), "second");
        let error = secrets.resolve("undeclared").expect_err("undeclared");
        assert_eq!(error.code, ScheduleErrorCode::BundleSecretForbidden);
        assert!(SecretEnvironment::default().is_empty());
    }
}
