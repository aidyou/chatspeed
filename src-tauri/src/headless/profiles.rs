//! Server-registered execution profile registry (Phase 2G+2H).
//!
//! A durable schedule request names an execution profile by *reference*. The
//! backend resolves that reference against the profile directory of its own
//! experiment domain, so a caller can never inject a host path, an image, a
//! network policy or a mount allowlist (AC-3/INV-4).
//!
//! The registry is deliberately filesystem-only and read-only: profiles are
//! provisioned by the operator (or by the Harbor adapter) before the headless
//! instance starts, and a request that names an unregistered profile fails
//! closed with `unknown_execution_profile`.

use crate::workflow::react::experiment_schedule::types::{
    ExecutionProfileV1, ScheduleError, ScheduleErrorCode,
};
use std::path::{Path, PathBuf};

/// Directory inside the experiment domain that holds the registered profiles.
pub const EXECUTION_PROFILE_DIR: &str = "execution-profiles";

fn profile_error(code: ScheduleErrorCode, message: impl Into<String>) -> ScheduleError {
    ScheduleError::new(code, message)
}

/// The profiles registered for one experiment domain.
#[derive(Debug, Clone)]
pub struct ExecutionProfileRegistry {
    root: PathBuf,
}

impl ExecutionProfileRegistry {
    /// Binds the registry to a domain root (the headless `--data-dir`).
    pub fn new(domain_root: impl AsRef<Path>) -> Self {
        Self {
            root: domain_root.as_ref().join(EXECUTION_PROFILE_DIR),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The profile references present on disk, sorted for determinism.
    pub fn list(&self) -> Result<Vec<String>, ScheduleError> {
        if !self.root.is_dir() {
            return Ok(Vec::new());
        }
        let entries = std::fs::read_dir(&self.root).map_err(|error| {
            profile_error(
                ScheduleErrorCode::UnknownExecutionProfile,
                format!(
                    "failed to read the execution profile directory '{}': {error}",
                    self.root.display()
                ),
            )
        })?;
        let mut refs = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                profile_error(
                    ScheduleErrorCode::UnknownExecutionProfile,
                    format!("failed to read an execution profile entry: {error}"),
                )
            })?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
                refs.push(stem.to_string());
            }
        }
        refs.sort();
        Ok(refs)
    }

    /// Loads and fully validates one registered profile.
    ///
    /// Every failure mode is a pre-dispatch rejection: an unknown reference, a
    /// reference that tries to escape the profile directory, a document that is
    /// not a strict `execution_profile.v1`, a profile whose declared ref does
    /// not match its file name, or a profile that violates the isolation rules.
    pub fn load(&self, profile_ref: &str) -> Result<ExecutionProfileV1, ScheduleError> {
        crate::workflow::react::experiment_schedule::types::is_valid_key(profile_ref)
            .then_some(())
            .ok_or_else(|| {
                profile_error(
                    ScheduleErrorCode::UnknownExecutionProfile,
                    format!("'{profile_ref}' is not a valid execution profile reference"),
                )
            })?;
        let path = self.root.join(format!("{profile_ref}.json"));
        let body = std::fs::read(&path).map_err(|error| {
            profile_error(
                ScheduleErrorCode::UnknownExecutionProfile,
                format!(
                    "no execution profile '{profile_ref}' is registered in this domain ({error})"
                ),
            )
        })?;
        let profile: ExecutionProfileV1 = serde_json::from_slice(&body).map_err(|error| {
            profile_error(
                ScheduleErrorCode::InvalidExecutionProfile,
                format!(
                    "execution profile '{profile_ref}' is not a valid strict document: {error}"
                ),
            )
        })?;
        if profile.profile_ref != profile_ref {
            return Err(profile_error(
                ScheduleErrorCode::InvalidExecutionProfile,
                format!(
                    "execution profile file '{profile_ref}.json' declares profile_ref '{}'",
                    profile.profile_ref
                ),
            ));
        }
        profile.validate()?;
        Ok(profile)
    }

    /// Confirms the requested bundle refs are permitted by the profile.
    ///
    /// A profile is the only place that decides which bundles a run may stage,
    /// so a request can never widen that set (AC-4).
    pub fn authorize_bundles(
        &self,
        profile: &ExecutionProfileV1,
        requested: &[String],
    ) -> Result<(), ScheduleError> {
        for bundle_ref in requested {
            if !profile
                .allowed_bundle_refs
                .iter()
                .any(|allowed| allowed == bundle_ref)
            {
                return Err(profile_error(
                    ScheduleErrorCode::BundleRefUnknown,
                    format!(
                        "bundle '{bundle_ref}' is not permitted by execution profile '{}'",
                        profile.profile_ref
                    ),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::experiment_schedule::types::{
        MountSpecV1, NetworkPolicyV1, OwnerKind, ResourceLimitsV1, EXECUTION_PROFILE_V1,
    };
    use tempfile::tempdir;

    fn profile_json(profile_ref: &str, owner: OwnerKind) -> String {
        let profile = ExecutionProfileV1 {
            schema_version: EXECUTION_PROFILE_V1.to_string(),
            profile_ref: profile_ref.to_string(),
            owner_kind: owner,
            base_repo_ref: "repo:primary".to_string(),
            base_revision: "refs/heads/main".to_string(),
            image_reference: match owner {
                OwnerKind::PersistentDocker => {
                    Some(format!("chatspeed/runner@sha256:{}", "a".repeat(64)))
                }
                _ => None,
            },
            network_policy: Some(NetworkPolicyV1 {
                mode: NetworkPolicyV1::MODE_NONE.to_string(),
                allow_hosts: Vec::new(),
            }),
            mounts: vec![MountSpecV1 {
                source_kind: MountSpecV1::SOURCE_WORKSPACE.to_string(),
                container_path: "/workspace".to_string(),
                read_only: false,
            }],
            resources: ResourceLimitsV1 {
                cpu_millis: 1000,
                memory_bytes: 1 << 30,
                pids: 128,
                no_new_privileges: true,
            },
            allowed_bundle_refs: vec!["smoke-tools".to_string()],
            input_patch_ref: None,
            input_patch_digest: None,
        };
        serde_json::to_string_pretty(&profile).expect("serialize")
    }

    fn write_profile(root: &Path, profile_ref: &str, body: &str) {
        let dir = root.join(EXECUTION_PROFILE_DIR);
        std::fs::create_dir_all(&dir).expect("create profile dir");
        std::fs::write(dir.join(format!("{profile_ref}.json")), body).expect("write profile");
    }

    #[test]
    fn a_registered_profile_loads_and_authorizes_its_bundles() {
        let directory = tempdir().expect("tempdir");
        write_profile(
            directory.path(),
            "smoke-local",
            &profile_json("smoke-local", OwnerKind::HostWorktree),
        );
        let registry = ExecutionProfileRegistry::new(directory.path());
        assert_eq!(
            registry.list().expect("list"),
            vec!["smoke-local".to_string()]
        );

        let profile = registry.load("smoke-local").expect("load");
        assert_eq!(profile.profile_ref, "smoke-local");
        registry
            .authorize_bundles(&profile, &["smoke-tools".to_string()])
            .expect("allowed bundle");
        let error = registry
            .authorize_bundles(&profile, &["other-bundle".to_string()])
            .expect_err("unlisted bundle");
        assert_eq!(error.code, ScheduleErrorCode::BundleRefUnknown);
    }

    #[test]
    fn unknown_and_escaping_references_fail_closed() {
        let directory = tempdir().expect("tempdir");
        let registry = ExecutionProfileRegistry::new(directory.path());
        let error = registry.load("missing").expect_err("unknown");
        assert_eq!(error.code, ScheduleErrorCode::UnknownExecutionProfile);
        // A traversal attempt never reaches the filesystem: the key alphabet
        // rejects it first.
        let error = registry.load("../etc/passwd").expect_err("escaping");
        assert_eq!(error.code, ScheduleErrorCode::UnknownExecutionProfile);
    }

    #[test]
    fn a_mismatched_or_invalid_profile_fails_closed() {
        let directory = tempdir().expect("tempdir");
        write_profile(
            directory.path(),
            "smoke-local",
            &profile_json("other-ref", OwnerKind::HostWorktree),
        );
        let registry = ExecutionProfileRegistry::new(directory.path());
        let error = registry.load("smoke-local").expect_err("ref mismatch");
        assert_eq!(error.code, ScheduleErrorCode::InvalidExecutionProfile);

        let directory = tempdir().expect("tempdir");
        write_profile(directory.path(), "smoke-local", "{ not json }");
        let registry = ExecutionProfileRegistry::new(directory.path());
        let error = registry.load("smoke-local").expect_err("invalid document");
        assert_eq!(error.code, ScheduleErrorCode::InvalidExecutionProfile);
    }

    #[test]
    fn an_unpinned_docker_profile_fails_closed_at_load_time() {
        let directory = tempdir().expect("tempdir");
        let body = profile_json("smoke-docker", OwnerKind::PersistentDocker)
            .replace("@sha256:", ":")
            .replace(&"a".repeat(64), "latest");
        write_profile(directory.path(), "smoke-docker", &body);
        let registry = ExecutionProfileRegistry::new(directory.path());
        let error = registry.load("smoke-docker").expect_err("unpinned image");
        assert_eq!(error.code, ScheduleErrorCode::ImageNotDigestPinned);
    }
}
