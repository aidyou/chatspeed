//! Server-registered promotion target registry (Phase 2I).
//!
//! A promotion request names a target by *reference*. The backend resolves that
//! reference against the promotion-target directory of its own experiment
//! domain, so a caller can never inject a branch, a repository path, a Git
//! identity, a canary programme or a threshold (AC-2/INV-4).
//!
//! The registry is deliberately filesystem-only and read-only: targets are
//! provisioned by the operator before the headless instance starts, and a
//! request that names an unregistered target fails closed with
//! `unknown_promotion_target`.
//!
//! Cross-document checks that need the other server registries also live here,
//! because this is the only place that holds both:
//!
//! - the referenced execution profile must be registered by the profile
//!   registry and must authorize the canary's bundle, and
//! - a `persistent_docker` profile must stay digest-pinned, which the profile
//!   registry already enforces at load time.

use crate::headless::profiles::ExecutionProfileRegistry;
use crate::workflow::react::experiment_promotion::policy::{is_digest_pinned, PromotionTargetV1};
use crate::workflow::react::experiment_promotion::types::{
    is_valid_key, PromotionError, PromotionErrorCode,
};
use crate::workflow::react::experiment_schedule::types::OwnerKind;
use std::path::{Path, PathBuf};

/// Directory inside the experiment domain that holds the registered targets.
pub const PROMOTION_TARGET_DIR: &str = "promotion-targets";

fn target_error(code: PromotionErrorCode, message: impl Into<String>) -> PromotionError {
    PromotionError::new(code, message)
}

/// The promotion targets registered for one experiment domain.
#[derive(Debug, Clone)]
pub struct PromotionTargetRegistry {
    root: PathBuf,
    profiles: ExecutionProfileRegistry,
}

impl PromotionTargetRegistry {
    /// Binds the registry to a domain root (the headless `--data-dir`).
    pub fn new(domain_root: impl AsRef<Path>) -> Self {
        let domain_root = domain_root.as_ref();
        Self {
            root: domain_root.join(PROMOTION_TARGET_DIR),
            profiles: ExecutionProfileRegistry::new(domain_root),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The target references present on disk, sorted for determinism.
    pub fn list(&self) -> Result<Vec<String>, PromotionError> {
        if !self.root.is_dir() {
            return Ok(Vec::new());
        }
        let entries = std::fs::read_dir(&self.root).map_err(|error| {
            target_error(
                PromotionErrorCode::UnknownPromotionTarget,
                format!(
                    "failed to read the promotion target directory '{}': {error}",
                    self.root.display()
                ),
            )
        })?;
        let mut refs = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                target_error(
                    PromotionErrorCode::UnknownPromotionTarget,
                    format!("failed to read a promotion target entry: {error}"),
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

    /// Loads and fully validates one registered target, including its
    /// cross-document bindings to the execution-profile registry.
    ///
    /// Every failure mode is a pre-effect rejection: an unknown reference, a
    /// reference that tries to escape the target directory, a document that is
    /// not a strict `promotion_target.v1`, a target whose declared ref does not
    /// match its file name, a document that violates the isolation rules, or a
    /// canary that names an unregistered profile/bundle.
    pub fn load(&self, target_ref: &str) -> Result<PromotionTargetV1, PromotionError> {
        if !is_valid_key(target_ref) {
            return Err(target_error(
                PromotionErrorCode::UnknownPromotionTarget,
                format!("'{target_ref}' is not a valid promotion target reference"),
            ));
        }
        let path = self.root.join(format!("{target_ref}.json"));
        let body = std::fs::read(&path).map_err(|error| {
            target_error(
                PromotionErrorCode::UnknownPromotionTarget,
                format!(
                    "no promotion target '{target_ref}' is registered in this domain ({error})"
                ),
            )
        })?;
        let target: PromotionTargetV1 = serde_json::from_slice(&body).map_err(|error| {
            target_error(
                PromotionErrorCode::InvalidPromotionTarget,
                format!("promotion target '{target_ref}' is not a valid strict document: {error}"),
            )
        })?;
        if target.target_ref != target_ref {
            return Err(target_error(
                PromotionErrorCode::InvalidPromotionTarget,
                format!(
                    "promotion target file '{target_ref}.json' declares target_ref '{}'",
                    target.target_ref
                ),
            ));
        }
        target.validate()?;
        self.authorize_canary(&target)?;
        Ok(target)
    }

    /// Confirms the canary's execution profile is registered and authorizes the
    /// canary's bundle, and that a container profile stays digest-pinned.
    ///
    /// This is the only place that can widen a target's reach, so it is checked
    /// at load time and again before the canary runs (TOCTOU defence).
    pub fn authorize_canary(&self, target: &PromotionTargetV1) -> Result<(), PromotionError> {
        let profile = self
            .profiles
            .load(&target.canary.execution_profile_ref)
            .map_err(|error| {
                target_error(
                    PromotionErrorCode::UnknownExecutionProfile,
                    format!(
                        "canary profile '{}' is not registered: {}",
                        target.canary.execution_profile_ref, error.message
                    ),
                )
            })?;
        self.profiles
            .authorize_bundles(&profile, std::slice::from_ref(&target.canary.bundle_ref))
            .map_err(|error| {
                target_error(
                    PromotionErrorCode::InvalidCanarySpec,
                    format!(
                        "canary bundle '{}' is not permitted by profile '{}': {}",
                        target.canary.bundle_ref, profile.profile_ref, error.message
                    ),
                )
            })?;
        if profile.owner_kind == OwnerKind::PersistentDocker {
            let image = profile.image_reference.as_deref().ok_or_else(|| {
                target_error(
                    PromotionErrorCode::InvalidCanarySpec,
                    "a persistent_docker canary profile must declare an image reference",
                )
            })?;
            if !is_digest_pinned(image) {
                return Err(target_error(
                    PromotionErrorCode::InvalidCanarySpec,
                    format!("canary image '{image}' is not pinned by sha256 digest"),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::experiment_promotion::policy::{
        CanaryStageSpecV1, MetricDirection, PromotionCanarySpecV1, PromotionMetricRuleV1,
        PromotionPolicyV1, PROMOTION_POLICY_V1, PROMOTION_TARGET_V1,
    };
    use crate::workflow::react::experiment_schedule::types::{
        ExecutionProfileV1, MountSpecV1, NetworkPolicyV1, ResourceLimitsV1, EXECUTION_PROFILE_V1,
    };
    use tempfile::tempdir;

    fn profile_json(profile_ref: &str, owner: OwnerKind, bundles: &[&str]) -> String {
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
            allowed_bundle_refs: bundles.iter().map(|bundle| bundle.to_string()).collect(),
            input_patch_ref: None,
            input_patch_digest: None,
        };
        serde_json::to_string_pretty(&profile).expect("serialize")
    }

    fn target_json(target_ref: &str, profile_ref: &str, bundle_ref: &str) -> String {
        let target = PromotionTargetV1 {
            schema_version: PROMOTION_TARGET_V1.to_string(),
            target_ref: target_ref.to_string(),
            base_repo_ref: "repo:primary".to_string(),
            branch_ref: "refs/heads/experiment/2i".to_string(),
            git_identity_name: "ChatSpeed Promotion".to_string(),
            git_identity_email: "promotion@chatspeed.local".to_string(),
            canary: PromotionCanarySpecV1 {
                execution_profile_ref: profile_ref.to_string(),
                bundle_ref: bundle_ref.to_string(),
                executable: "./tools/canary".to_string(),
                stages: vec![CanaryStageSpecV1 {
                    stage_id: "stage-1".to_string(),
                    args: vec!["--task".to_string(), "task-a".to_string()],
                    metric: "verdict_score".to_string(),
                    direction: MetricDirection::HigherIsBetter,
                    min_improvement: 0.1,
                    max_regression: 0.0,
                    min_samples: 4,
                    required: true,
                }],
                timeout_ms: 60_000,
                max_output_bytes: 65_536,
            },
            policy: PromotionPolicyV1 {
                schema_version: PROMOTION_POLICY_V1.to_string(),
                policy_ref: "default".to_string(),
                require_verdict_pass: true,
                require_safety_pass: true,
                require_infra_pass: true,
                allow_unknown_cost: false,
                max_committed_micros: 1_000_000,
                metrics: vec![PromotionMetricRuleV1 {
                    metric: "verdict_score".to_string(),
                    direction: MetricDirection::HigherIsBetter,
                    min_improvement: 0.1,
                    max_regression: 0.0,
                    min_samples: 4,
                    required: true,
                }],
            },
        };
        serde_json::to_string_pretty(&target).expect("serialize")
    }

    fn write_domain(root: &Path, profile: &str, target: Option<&str>) {
        let profiles = root.join(crate::headless::profiles::EXECUTION_PROFILE_DIR);
        std::fs::create_dir_all(&profiles).expect("profile dir");
        std::fs::write(profiles.join("smoke-local.json"), profile).expect("write profile");
        let targets = root.join(PROMOTION_TARGET_DIR);
        std::fs::create_dir_all(&targets).expect("target dir");
        if let Some(target) = target {
            std::fs::write(targets.join("local-dev.json"), target).expect("write target");
        }
    }

    #[test]
    fn a_registered_target_loads_with_its_profile_and_bundle() {
        let directory = tempdir().expect("tempdir");
        write_domain(
            directory.path(),
            &profile_json("smoke-local", OwnerKind::HostWorktree, &["smoke-tools"]),
            Some(&target_json("local-dev", "smoke-local", "smoke-tools")),
        );
        let registry = PromotionTargetRegistry::new(directory.path());
        assert_eq!(
            registry.list().expect("list"),
            vec!["local-dev".to_string()]
        );
        let target = registry.load("local-dev").expect("load");
        assert_eq!(target.target_ref, "local-dev");
        assert_eq!(target.branch_ref, "refs/heads/experiment/2i");
    }

    #[test]
    fn unknown_escaping_and_unregistered_documents_fail_closed() {
        let directory = tempdir().expect("tempdir");
        write_domain(
            directory.path(),
            &profile_json("smoke-local", OwnerKind::HostWorktree, &["smoke-tools"]),
            None,
        );
        let registry = PromotionTargetRegistry::new(directory.path());
        assert_eq!(
            registry.load("missing").expect_err("unknown").code,
            PromotionErrorCode::UnknownPromotionTarget
        );
        assert_eq!(
            registry.load("../etc/passwd").expect_err("escape").code,
            PromotionErrorCode::UnknownPromotionTarget
        );

        // A target that declares a profile the server never registered.
        write_domain(
            directory.path(),
            &profile_json("smoke-local", OwnerKind::HostWorktree, &["smoke-tools"]),
            Some(&target_json("local-dev", "other-profile", "smoke-tools")),
        );
        assert_eq!(
            registry.load("local-dev").expect_err("profile").code,
            PromotionErrorCode::UnknownExecutionProfile
        );

        // A target whose canary names a bundle the profile does not allow.
        write_domain(
            directory.path(),
            &profile_json("smoke-local", OwnerKind::HostWorktree, &["other-bundle"]),
            Some(&target_json("local-dev", "smoke-local", "smoke-tools")),
        );
        assert_eq!(
            registry.load("local-dev").expect_err("bundle").code,
            PromotionErrorCode::InvalidCanarySpec
        );
    }

    #[test]
    fn a_mismatched_or_non_strict_target_document_is_rejected() {
        let directory = tempdir().expect("tempdir");
        write_domain(
            directory.path(),
            &profile_json("smoke-local", OwnerKind::HostWorktree, &["smoke-tools"]),
            Some(&target_json("other-ref", "smoke-local", "smoke-tools")),
        );
        let registry = PromotionTargetRegistry::new(directory.path());
        assert_eq!(
            registry.load("local-dev").expect_err("ref mismatch").code,
            PromotionErrorCode::InvalidPromotionTarget
        );

        write_domain(
            directory.path(),
            &profile_json("smoke-local", OwnerKind::HostWorktree, &["smoke-tools"]),
            Some("{ not json }"),
        );
        assert_eq!(
            registry.load("local-dev").expect_err("invalid").code,
            PromotionErrorCode::InvalidPromotionTarget
        );

        // A caller-forbidden field (a branch override smuggled into the target)
        // is rejected by the strict schema.
        let injected = target_json("local-dev", "smoke-local", "smoke-tools").replace(
            "\"target_ref\"",
            "\"branch_override\": \"refs/heads/main\", \"target_ref\"",
        );
        write_domain(
            directory.path(),
            &profile_json("smoke-local", OwnerKind::HostWorktree, &["smoke-tools"]),
            Some(&injected),
        );
        assert_eq!(
            registry.load("local-dev").expect_err("unknown field").code,
            PromotionErrorCode::InvalidPromotionTarget
        );
    }
}
