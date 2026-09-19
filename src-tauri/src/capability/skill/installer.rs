//! Multi-target Agent Skill installation.
//!
//! Apply takes a frozen [`SkillInstallPlan`] and touches each selected target
//! independently. Two rules dominate the design:
//!
//! * nothing is ever overwritten — an ordinary same-name directory is skipped,
//!   and a suspicious path (symlink, non-directory) blocks that target
//!   (AC-4/INV-6);
//! * every target effect is journaled *before* it happens and observed *after*,
//!   so a crash leaves a provable intent rather than a guess (AC-2/INV-8).
//!
//! Commit is a sibling temporary directory plus a rename on the same
//! filesystem, so an interrupted copy can never be mistaken for an install.

use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::capability::error::CapabilityError;
use crate::capability::operation::now_ms;
use crate::capability::repository::CapabilityRepository;
use crate::capability::skill::manifest::{manifest_digest, verify_file_manifest};
use crate::capability::skill::ownership::{self, OwnershipMarker};
use crate::capability::skill::plan::SkillInstallPlan;
use crate::capability::targets::{resolve_target_or_error, SkillTargetId, TargetEnvironment};
use crate::capability::types::{EffectOutcome, SkillInstallation, SkillInstallationState};

/// What happened on one target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetOutcomeStatus {
    /// The skill was installed into this target.
    Installed,
    /// This exact install is already present.
    AlreadyInstalled,
    /// A same-name directory exists that ChatSpeed does not manage.
    SkippedExisting,
    /// The path is suspicious, so nothing was written.
    Blocked,
    /// The target has no verified directory.
    Unsupported,
    /// The target was attempted and failed.
    Failed,
}

/// The result of one target.
#[derive(Debug, Clone, Serialize)]
pub struct TargetOutcome {
    pub target_id: String,
    pub status: TargetOutcomeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl TargetOutcome {
    /// The journal outcome this result proves.
    pub fn effect_outcome(&self) -> EffectOutcome {
        match self.status {
            TargetOutcomeStatus::Installed | TargetOutcomeStatus::AlreadyInstalled => {
                EffectOutcome::Applied
            }
            TargetOutcomeStatus::SkippedExisting => EffectOutcome::Skipped,
            TargetOutcomeStatus::Blocked | TargetOutcomeStatus::Unsupported => {
                EffectOutcome::Blocked
            }
            TargetOutcomeStatus::Failed => EffectOutcome::Failed,
        }
    }
}

/// The per-target results of one install.
#[derive(Debug, Clone, Serialize)]
pub struct InstallSummary {
    pub plan_id: String,
    pub skill_name: String,
    pub content_digest: String,
    pub outcomes: Vec<TargetOutcome>,
}

impl InstallSummary {
    pub fn installed(&self) -> Vec<&TargetOutcome> {
        self.outcomes
            .iter()
            .filter(|outcome| outcome.status == TargetOutcomeStatus::Installed)
            .collect()
    }

    pub fn failures(&self) -> Vec<&TargetOutcome> {
        self.outcomes
            .iter()
            .filter(|outcome| outcome.status == TargetOutcomeStatus::Failed)
            .collect()
    }

    /// Whether at least one target refused for a reason the user must fix.
    pub fn has_refusals(&self) -> bool {
        self.outcomes.iter().any(|outcome| {
            matches!(
                outcome.status,
                TargetOutcomeStatus::Blocked | TargetOutcomeStatus::Failed
            )
        })
    }
}

/// Applies install plans to the registered Skill targets.
pub struct SkillInstaller {
    environment: TargetEnvironment,
}

impl SkillInstaller {
    pub fn new(environment: TargetEnvironment) -> Self {
        Self { environment }
    }

    /// Installs a frozen plan, target by target.
    ///
    /// This never returns `Err` for a per-target problem: a blocked or failed
    /// target is a structured outcome, so the caller can report which targets
    /// succeeded instead of losing the whole operation.
    pub fn apply(
        &self,
        plan: &SkillInstallPlan,
        repository: &CapabilityRepository,
        operation_id: &str,
    ) -> Result<InstallSummary, CapabilityError> {
        plan.verify_frozen()?;

        let mut outcomes = Vec::with_capacity(plan.targets.len());
        for target_id in &plan.targets {
            let effect_key = format!("skill.install:{target_id}");
            let intent = serde_json::json!({
                "skill_name": plan.skill_name,
                "content_digest": plan.content_digest,
                "checker_version": plan.checker_version,
            });
            repository.record_effect_intent(
                operation_id,
                &effect_key,
                Some(target_id.as_str()),
                Some(&intent),
            )?;

            let outcome = self.apply_one(plan, target_id, repository, operation_id);
            let observation = serde_json::json!({
                "status": outcome.status,
                "install_path": outcome.install_path,
                "installation_id": outcome.installation_id,
            });
            repository.record_effect_outcome(
                operation_id,
                &effect_key,
                outcome.effect_outcome(),
                Some(&observation),
            )?;
            outcomes.push(outcome);
        }

        Ok(InstallSummary {
            plan_id: plan.plan_id.clone(),
            skill_name: plan.skill_name.clone(),
            content_digest: plan.content_digest.clone(),
            outcomes,
        })
    }

    fn apply_one(
        &self,
        plan: &SkillInstallPlan,
        target_id: &str,
        repository: &CapabilityRepository,
        operation_id: &str,
    ) -> TargetOutcome {
        let resolved = match resolve_target_or_error(target_id, &self.environment) {
            Ok((_, path)) => path,
            Err(error) => {
                return TargetOutcome {
                    target_id: target_id.to_string(),
                    status: TargetOutcomeStatus::Unsupported,
                    install_path: None,
                    installation_id: None,
                    detail: Some(error.redacted_message()),
                };
            }
        };

        let install_path = resolved.join(&plan.skill_name);
        match self.inspect_existing(&install_path, target_id, &plan.skill_name, repository) {
            Ok(Some(outcome)) => return outcome,
            Ok(None) => {}
            Err(error) => {
                return TargetOutcome {
                    target_id: target_id.to_string(),
                    status: TargetOutcomeStatus::Blocked,
                    install_path: Some(install_path.to_string_lossy().to_string()),
                    installation_id: None,
                    detail: Some(error.redacted_message()),
                };
            }
        }

        match self.commit(plan, target_id, &resolved, &install_path, repository, operation_id) {
            Ok(installation_id) => TargetOutcome {
                target_id: target_id.to_string(),
                status: TargetOutcomeStatus::Installed,
                install_path: Some(install_path.to_string_lossy().to_string()),
                installation_id: Some(installation_id),
                detail: None,
            },
            Err(error) => TargetOutcome {
                target_id: target_id.to_string(),
                status: TargetOutcomeStatus::Failed,
                install_path: Some(install_path.to_string_lossy().to_string()),
                installation_id: None,
                detail: Some(error.redacted_message()),
            },
        }
    }

    /// Decides what to do about a path that is already there.
    ///
    /// `Ok(Some(outcome))` means "stop, this target is settled"; `Ok(None)`
    /// means "the path is free and the install may proceed".
    fn inspect_existing(
        &self,
        install_path: &Path,
        target_id: &str,
        skill_name: &str,
        repository: &CapabilityRepository,
    ) -> Result<Option<TargetOutcome>, CapabilityError> {
        let metadata = match std::fs::symlink_metadata(install_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(CapabilityError::internal(format!(
                    "failed to inspect the install path: {error}"
                )));
            }
        };

        let settle = |status: TargetOutcomeStatus, detail: Option<String>| {
            Ok(Some(TargetOutcome {
                target_id: target_id.to_string(),
                status,
                install_path: Some(install_path.to_string_lossy().to_string()),
                installation_id: None,
                detail,
            }))
        };

        if metadata.file_type().is_symlink() {
            return settle(
                TargetOutcomeStatus::Blocked,
                Some("the install path is a symbolic link".to_string()),
            );
        }
        if !metadata.is_dir() {
            return settle(
                TargetOutcomeStatus::Blocked,
                Some("the install path exists and is not a directory".to_string()),
            );
        }

        let recorded = repository.get_installation(target_id, skill_name)?;
        let Some(recorded) = recorded else {
            return settle(
                TargetOutcomeStatus::SkippedExisting,
                Some("a same-name directory already exists".to_string()),
            );
        };

        if ownership::has_proof(install_path, &recorded)?
            && verify_file_manifest(install_path, &recorded.file_manifest)?.is_match()
        {
            return settle(
                TargetOutcomeStatus::AlreadyInstalled,
                Some("this exact installation is already present".to_string()),
            );
        }

        // A managed directory that no longer matches its own proof is never
        // overwritten; drift is reported and left for the user or doctor.
        settle(
            TargetOutcomeStatus::SkippedExisting,
            Some("an existing managed directory has drifted and is never overwritten".to_string()),
        )
    }

    /// Copies the frozen content into a sibling temp directory, records the
    /// ownership row, then renames the temp directory into place.
    fn commit(
        &self,
        plan: &SkillInstallPlan,
        target_id: &str,
        target_root: &Path,
        install_path: &Path,
        repository: &CapabilityRepository,
        operation_id: &str,
    ) -> Result<String, CapabilityError> {
        std::fs::create_dir_all(target_root).map_err(|error| {
            CapabilityError::internal(format!("failed to create the target directory: {error}"))
        })?;

        let nonce = uuid::Uuid::now_v7().simple().to_string();
        let temp = target_root.join(format!(".{}.cs-install-{}", plan.skill_name, nonce));
        let result = self.copy_into(&temp, plan, target_id, install_path, repository, operation_id, &nonce);
        match result {
            Ok(installation_id) => Ok(installation_id),
            Err(error) => {
                // A failed commit must not leave a half-copied tree behind.
                let _ = std::fs::remove_dir_all(&temp);
                Err(error)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn copy_into(
        &self,
        temp: &Path,
        plan: &SkillInstallPlan,
        target_id: &str,
        install_path: &Path,
        repository: &CapabilityRepository,
        operation_id: &str,
        nonce: &str,
    ) -> Result<String, CapabilityError> {
        std::fs::create_dir_all(temp).map_err(|error| {
            CapabilityError::internal(format!("failed to create the staging directory: {error}"))
        })?;

        for entry in &plan.files {
            let from = plan.frozen_root.join(&entry.path);
            let to = temp.join(&entry.path);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    CapabilityError::internal(format!("failed to create a directory: {error}"))
                })?;
            }
            std::fs::copy(&from, &to).map_err(|error| {
                CapabilityError::internal(format!("failed to copy the skill content: {error}"))
            })?;
            let bytes = std::fs::read(&to).map_err(|error| {
                CapabilityError::internal(format!("failed to verify the copied content: {error}"))
            })?;
            if hex::encode(Sha256::digest(&bytes)) != entry.sha256 {
                return Err(CapabilityError::refused(format!(
                    "the copied content does not match the plan at '{}'",
                    entry.path
                )));
            }
        }

        let created_at_ms = now_ms();
        let manifest_digest = manifest_digest(&plan.files).ok_or_else(|| {
            CapabilityError::refused("the plan has no content to install")
        })?;
        let installation = SkillInstallation {
            installation_id: format!("ins-{}", uuid::Uuid::now_v7()),
            skill_name: plan.skill_name.clone(),
            target_id: target_id.to_string(),
            install_path: install_path.to_string_lossy().to_string(),
            source_kind: plan.source_kind.clone(),
            source_ref: plan.source_ref.clone(),
            checker_version: plan.checker_version.clone(),
            verdict: plan.verdict.clone(),
            content_digest: plan.content_digest.clone(),
            file_manifest: plan.files.clone(),
            marker_nonce: nonce.to_string(),
            manifest_digest,
            state: SkillInstallationState::Installing,
            operation_id: Some(operation_id.to_string()),
            created_at_ms,
            updated_at_ms: created_at_ms,
        };

        // The marker travels with the content inside the temp directory, so
        // the directory is never visible without its own proof.
        ownership::write_marker(temp, &OwnershipMarker::from_installation(&installation))?;

        // The durable intent is written before the rename: a crash after this
        // point is recoverable, a crash before it changed nothing.
        let installation_id = repository.upsert_installation(&installation)?.installation_id;

        std::fs::rename(temp, install_path).map_err(|error| {
            CapabilityError::internal(format!("failed to publish the installed skill: {error}"))
        })?;

        repository.set_installation_state(&installation_id, SkillInstallationState::Installed)?;
        Ok(installation_id)
    }

    /// The default target selection, resolved to a concrete id list.
    pub fn default_targets() -> Vec<SkillTargetId> {
        crate::capability::targets::default_target_selection()
    }

    /// The target root for one id, when it is supported.
    pub fn target_root(&self, target_id: SkillTargetId) -> Result<PathBuf, CapabilityError> {
        resolve_target_or_error(target_id.as_str(), &self.environment).map(|(_, path)| path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::skill::checker::check_directory;
    use crate::capability::skill::source::SkillSource;
    use crate::db::MainStore;
    use std::sync::Arc;
    use tempfile::TempDir;

    struct Fixture {
        _temp: TempDir,
        environment: TargetEnvironment,
        repository: CapabilityRepository,
        content: PathBuf,
        frozen: PathBuf,
    }

    fn fixture() -> Fixture {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path().join("home");
        let chatspeed = temp.path().join("chatspeed-home");
        std::fs::create_dir_all(&home).expect("create home");

        let content = temp.path().join("content");
        std::fs::create_dir_all(&content).expect("create content");
        std::fs::write(content.join("SKILL.md"), "---\nname: demo\n---\n\n# demo\n")
            .expect("write skill");
        std::fs::write(content.join("notes.md"), "prose").expect("write notes");

        let store = Arc::new(MainStore::new(":memory:").expect("in-memory store"));
        let repository = CapabilityRepository::new(store);
        let environment = TargetEnvironment::injected(home, chatspeed);

        Fixture {
            frozen: temp.path().join("frozen"),
            _temp: temp,
            environment,
            repository,
            content,
        }
    }

    fn plan(fixture: &Fixture, targets: &[SkillTargetId]) -> SkillInstallPlan {
        let report = check_directory(&fixture.content).expect("check");
        let source = SkillSource::LocalDirectory {
            path: fixture.content.to_string_lossy().to_string(),
        };
        SkillInstallPlan::build(
            &source,
            &report,
            &fixture.content,
            targets,
            fixture.frozen.clone(),
        )
        .expect("plan")
    }

    /// Opens a real operation row: the effect journal has a foreign key onto
    /// it, so a target effect can never be recorded without its operation.
    fn operation(fixture: &Fixture, idempotency_key: &str) -> String {
        use crate::capability::types::{CapabilityKind, OperationBegin, OperationRequest};
        let request = OperationRequest {
            capability: CapabilityKind::Skill,
            operation_kind: "skill.install".to_string(),
            actor_scope: "test".to_string(),
            idempotency_key: idempotency_key.to_string(),
            request: serde_json::json!({ "skill_name": "demo" }),
            resource_key: "skill:demo".to_string(),
        };
        match fixture.repository.begin(&request).expect("begin operation") {
            OperationBegin::Started(operation) => operation.operation_id,
            OperationBegin::Replay(operation) => operation.operation_id,
        }
    }

    #[test]
    fn the_default_selection_is_only_chatspeed() {
        let targets = SkillInstaller::default_targets();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0], SkillTargetId::Chatspeed);
    }

    #[test]
    fn a_supported_target_without_an_existing_directory_echoes_its_root() {
        let fixture = fixture();
        let installer = SkillInstaller::new(fixture.environment.clone());
        let root = installer
            .target_root(SkillTargetId::Chatspeed)
            .expect("chatspeed root");
        assert_eq!(
            root,
            fixture
                .environment
                .chatspeed_skills_dir()
                .expect("skills dir")
        );
    }

    #[test]
    fn an_unsupported_target_is_never_created() {
        let fixture = fixture();
        let installer = SkillInstaller::new(fixture.environment.clone());
        let out = installer.apply_one(
            &plan(&fixture, &[SkillTargetId::Chatspeed]),
            "cursor",
            &fixture.repository,
            "op-1",
        );
        assert_eq!(out.status, TargetOutcomeStatus::Unsupported);
        assert!(out.install_path.is_none());
    }

    #[test]
    fn installed_skills_are_recorded_and_a_plain_same_name_directory_is_skipped() {
        let fixture = fixture();
        let plan = plan(&fixture, &[SkillTargetId::Chatspeed]);
        let installer = SkillInstaller::new(fixture.environment.clone());
        let summary = installer
            .apply(&plan, &fixture.repository, &operation(&fixture, "key-1"))
            .expect("apply");

        assert_eq!(summary.outcomes.len(), 1);
        assert_eq!(summary.outcomes[0].status, TargetOutcomeStatus::Installed);

        let install_path = PathBuf::from(summary.outcomes[0].install_path.clone().unwrap());
        assert!(install_path.join("SKILL.md").is_file());
        assert!(install_path.join(".chatspeed-skill.json").is_file());

        // The row exists and the drift check sees the marker-free picture.
        let row = fixture
            .repository
            .get_installation("chatspeed", "demo")
            .expect("lookup")
            .expect("row");
        assert_eq!(row.state, SkillInstallationState::Installed);
        assert!(ownership::has_proof(&install_path, &row).expect("proof"));

        // Applying the same plan again is idempotent.
        let again = installer
            .apply(&plan, &fixture.repository, &operation(&fixture, "key-2"))
            .expect("apply again");
        assert_eq!(again.outcomes[0].status, TargetOutcomeStatus::AlreadyInstalled);

        // A different skill with the same name in the same target is skipped.
        let mut other = plan.clone();
        other.skill_name = "demo".to_string();
        std::fs::remove_file(install_path.join(".chatspeed-skill.json")).expect("remove marker");
        let skipped = installer
            .apply(&other, &fixture.repository, &operation(&fixture, "key-3"))
            .expect("apply third");
        assert_eq!(
            skipped.outcomes[0].status,
            TargetOutcomeStatus::SkippedExisting
        );
        // The existing content was not touched.
        assert_eq!(
            std::fs::read_to_string(install_path.join("notes.md")).expect("read"),
            "prose"
        );
    }

    #[test]
    fn a_directory_without_an_ownership_row_is_skipped_and_kept() {
        let fixture = fixture();
        let plan = plan(&fixture, &[SkillTargetId::Chatspeed]);
        let installer = SkillInstaller::new(fixture.environment.clone());

        let skills_dir = fixture
            .environment
            .chatspeed_skills_dir()
            .expect("skills dir");
        std::fs::create_dir_all(skills_dir.join("demo")).expect("create existing");
        std::fs::write(skills_dir.join("demo/handwritten.md"), "mine").expect("write");

        let summary = installer
            .apply(&plan, &fixture.repository, &operation(&fixture, "key-1"))
            .expect("apply");
        assert_eq!(
            summary.outcomes[0].status,
            TargetOutcomeStatus::SkippedExisting
        );
        assert!(skills_dir.join("demo/handwritten.md").is_file());
        assert!(!skills_dir.join("demo/SKILL.md").exists());
    }

    #[test]
    fn a_symbolic_link_install_path_blocks_that_target() {
        #[cfg(unix)]
        {
            let fixture = fixture();
            let plan = plan(&fixture, &[SkillTargetId::Chatspeed]);
            let installer = SkillInstaller::new(fixture.environment.clone());

            let skills_dir = fixture
                .environment
                .chatspeed_skills_dir()
                .expect("skills dir");
            std::fs::create_dir_all(&skills_dir).expect("create skills dir");
            let outside = fixture._temp.path().join("outside");
            std::fs::create_dir_all(&outside).expect("create outside");
            std::os::unix::fs::symlink(&outside, skills_dir.join("demo")).expect("symlink");

            let summary = installer
                .apply(&plan, &fixture.repository, &operation(&fixture, "key-1"))
                .expect("apply");
            assert_eq!(summary.outcomes[0].status, TargetOutcomeStatus::Blocked);
            assert!(outside.read_dir().expect("read outside").next().is_none());
        }
    }

    #[test]
    fn apply_refuses_a_plan_whose_frozen_content_changed() {
        let fixture = fixture();
        let plan = plan(&fixture, &[SkillTargetId::Chatspeed]);
        let installer = SkillInstaller::new(fixture.environment.clone());

        std::fs::write(plan.frozen_root.join("notes.md"), "tampered").expect("tamper");
        let error = installer
            .apply(&plan, &fixture.repository, "op-1")
            .err()
            .expect("a tampered plan must not be applied");
        assert_eq!(error.code(), crate::capability::error::code::REFUSED);
    }
}
