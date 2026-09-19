//! Ownership-bound Agent Skill uninstallation.
//!
//! A directory is only removed when ChatSpeed can prove all of:
//!
//! * a durable installation row exists for this `(target, name)`;
//! * the row was created for exactly this path;
//! * the in-directory ownership marker matches the row;
//! * the content still matches the recorded manifest (no drift);
//! * the name is not reserved.
//!
//! Anything else is refused with zero deletion (AC-7/INV-6). The directory is
//! first renamed to a sibling quarantine inside the same target, then the row
//! is finalized and the quarantine is removed, so an interrupted uninstall is
//! always recognizable rather than half-deleted.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::capability::error::CapabilityError;
use crate::capability::repository::CapabilityRepository;
use crate::capability::skill::manifest::{verify_file_manifest, ManifestVerdict};
use crate::capability::skill::ownership;
use crate::capability::skill_inventory::RESERVED_SKILL_NAMES;
use crate::capability::targets::{resolve_target_or_error, TargetEnvironment};
use crate::capability::types::{EffectOutcome, SkillInstallation, SkillInstallationState};

/// What an uninstall did on one target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UninstallOutcomeStatus {
    /// The managed directory was removed.
    Removed,
    /// The directory is gone but the ownership row needed finalizing.
    Finalized,
    /// Nothing was removed, with a reason.
    Refused,
    /// There is nothing installed under this name.
    NotFound,
}

/// The result of one uninstall.
#[derive(Debug, Clone, Serialize)]
pub struct UninstallOutcome {
    pub target_id: String,
    pub skill_name: String,
    pub status: UninstallOutcomeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quarantine_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl UninstallOutcome {
    pub fn effect_outcome(&self) -> EffectOutcome {
        match self.status {
            UninstallOutcomeStatus::Removed | UninstallOutcomeStatus::Finalized => EffectOutcome::Applied,
            UninstallOutcomeStatus::Refused => EffectOutcome::Blocked,
            UninstallOutcomeStatus::NotFound => EffectOutcome::Skipped,
        }
    }
}

/// Removes managed Agent Skill installations.
pub struct SkillUninstaller {
    environment: TargetEnvironment,
}

impl SkillUninstaller {
    pub fn new(environment: TargetEnvironment) -> Self {
        Self { environment }
    }

    /// Uninstalls one skill from one target, journaling intent and observation.
    pub fn uninstall(
        &self,
        repository: &CapabilityRepository,
        operation_id: &str,
        target_id: &str,
        skill_name: &str,
    ) -> Result<UninstallOutcome, CapabilityError> {
        let effect_key = format!("skill.uninstall:{target_id}");
        let intent = serde_json::json!({ "skill_name": skill_name });
        repository.record_effect_intent(
            operation_id,
            &effect_key,
            Some(target_id),
            Some(&intent),
        )?;

        let outcome = self.uninstall_one(repository, target_id, skill_name);
        let observation = serde_json::json!({
            "status": outcome.status,
            "install_path": outcome.install_path,
            "quarantine_path": outcome.quarantine_path,
            "detail": outcome.detail,
        });
        repository.record_effect_outcome(
            operation_id,
            &effect_key,
            outcome.effect_outcome(),
            Some(&observation),
        )?;
        Ok(outcome)
    }

    /// Idempotently converges the on-disk state of one *owned* installation for
    /// crash recovery, using only proven durable evidence. It never deletes an
    /// unproven or drifted directory and never overwrites user content (INV-6).
    ///
    /// Returns the outcome only when it actually advanced durable state; `None`
    /// means there was nothing proven to converge (leave it for the next pass).
    pub fn reconcile_owned_directory(
        &self,
        repository: &CapabilityRepository,
        target_id: &str,
        skill_name: &str,
    ) -> Result<Option<UninstallOutcome>, CapabilityError> {
        if RESERVED_SKILL_NAMES.contains(&skill_name) {
            return Ok(None);
        }
        let Some(recorded) = repository.get_installation(target_id, skill_name)? else {
            return Ok(None);
        };
        let install_path = PathBuf::from(&recorded.install_path);
        match recorded.state {
            // A quarantine that never finalized: delete only the directory
            // ChatSpeed moved aside and record the row as removed.
            SkillInstallationState::Quarantined => Ok(Some(self.finish_quarantine(
                repository,
                &recorded,
                &install_path,
            ))),
            // An install whose rename did not finish: if the committed content
            // and marker prove the install, record it Installed; otherwise leave
            // it (never delete an unproven or partial directory here).
            SkillInstallationState::Installing => {
                if install_path.exists()
                    && ownership::has_proof(&install_path, &recorded)?
                    && matches!(
                        verify_file_manifest(&install_path, &recorded.file_manifest)?,
                        ManifestVerdict::Match
                    )
                {
                    repository.set_installation_state(
                        &recorded.installation_id,
                        SkillInstallationState::Installed,
                    )?;
                    return Ok(Some(UninstallOutcome {
                        target_id: target_id.to_string(),
                        skill_name: skill_name.to_string(),
                        status: UninstallOutcomeStatus::Finalized,
                        install_path: Some(recorded.install_path.clone()),
                        quarantine_path: None,
                        detail: Some(
                            "an interrupted install whose content matches its proof was finalized"
                                .to_string(),
                        ),
                    }));
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn refuse(
        &self,
        target_id: &str,
        skill_name: &str,
        install_path: Option<&Path>,
        detail: impl Into<String>,
    ) -> UninstallOutcome {
        UninstallOutcome {
            target_id: target_id.to_string(),
            skill_name: skill_name.to_string(),
            status: UninstallOutcomeStatus::Refused,
            install_path: install_path.map(|path| path.to_string_lossy().to_string()),
            quarantine_path: None,
            detail: Some(detail.into()),
        }
    }

    fn uninstall_one(
        &self,
        repository: &CapabilityRepository,
        target_id: &str,
        skill_name: &str,
    ) -> UninstallOutcome {
        if RESERVED_SKILL_NAMES.contains(&skill_name) {
            return self.refuse(
                target_id,
                skill_name,
                None,
                "this name is reserved and is never uninstalled",
            );
        }

        let target_root = match resolve_target_or_error(target_id, &self.environment) {
            Ok((_, path)) => path,
            Err(error) => {
                return self.refuse(target_id, skill_name, None, error.redacted_message());
            }
        };
        let install_path = target_root.join(skill_name);

        let recorded = match repository.get_installation(target_id, skill_name) {
            Ok(recorded) => recorded,
            Err(error) => {
                return self.refuse(
                    target_id,
                    skill_name,
                    Some(&install_path),
                    error.redacted_message(),
                );
            }
        };
        let Some(recorded) = recorded else {
            return UninstallOutcome {
                target_id: target_id.to_string(),
                skill_name: skill_name.to_string(),
                status: UninstallOutcomeStatus::NotFound,
                install_path: Some(install_path.to_string_lossy().to_string()),
                quarantine_path: None,
                detail: Some("no recorded installation owns this name".to_string()),
            };
        };

        // A row that already says "removed" or "quarantined" is metadata to
        // reconcile, never a fresh deletion.
        match recorded.state {
            SkillInstallationState::Installed => {}
            SkillInstallationState::Quarantined => {
                return self.finish_quarantine(repository, &recorded, &install_path);
            }
            SkillInstallationState::Removed => {
                return UninstallOutcome {
                    target_id: target_id.to_string(),
                    skill_name: skill_name.to_string(),
                    status: UninstallOutcomeStatus::NotFound,
                    install_path: Some(install_path.to_string_lossy().to_string()),
                    quarantine_path: None,
                    detail: Some("this installation was already removed".to_string()),
                };
            }
            other => {
                return self.refuse(
                    target_id,
                    skill_name,
                    Some(&install_path),
                    format!("the installation is in state '{}' and is not uninstallable", other.as_str()),
                );
            }
        }

        if recorded.install_path != install_path.to_string_lossy() {
            return self.refuse(
                target_id,
                skill_name,
                Some(&install_path),
                "the recorded installation belongs to a different path",
            );
        }
        if !install_path.exists() {
            // The directory is already gone; finalize the row instead of
            // guessing about a deletion that already happened.
            return self.finalize_row(repository, &recorded, None);
        }

        match ownership::has_proof(&install_path, &recorded) {
            Ok(true) => {}
            Ok(false) => {
                return self.refuse(
                    target_id,
                    skill_name,
                    Some(&install_path),
                    "the ownership marker does not match the recorded installation",
                );
            }
            Err(error) => {
                return self.refuse(
                    target_id,
                    skill_name,
                    Some(&install_path),
                    error.redacted_message(),
                );
            }
        }

        match verify_file_manifest(&install_path, &recorded.file_manifest) {
            Ok(ManifestVerdict::Match) => {}
            Ok(_) => {
                return self.refuse(
                    target_id,
                    skill_name,
                    Some(&install_path),
                    "the installed content has drifted and is never deleted",
                );
            }
            Err(error) => {
                return self.refuse(
                    target_id,
                    skill_name,
                    Some(&install_path),
                    error.redacted_message(),
                );
            }
        }

        // Same-filesystem rename inside the target root: atomic, and it never
        // depends on app data living on the same device as the target.
        let quarantine = target_root.join(format!(
            ".{}.cs-quarantine-{}",
            skill_name,
            uuid::Uuid::now_v7().simple()
        ));
        if let Err(error) = std::fs::rename(&install_path, &quarantine) {
            return self.refuse(
                target_id,
                skill_name,
                Some(&install_path),
                format!("failed to quarantine the installed skill: {error}"),
            );
        }

        match repository.set_installation_state(&recorded.installation_id, SkillInstallationState::Quarantined)
        {
            Ok(_) => {}
            Err(error) => {
                // The content moved but the row did not: restore the directory
                // so the user's skill is not left in a hidden quarantine.
                let _ = std::fs::rename(&quarantine, &install_path);
                return self.refuse(
                    target_id,
                    skill_name,
                    Some(&install_path),
                    error.redacted_message(),
                );
            }
        }

        let mut outcome = self.finalize_row(repository, &recorded, Some(&quarantine));
        outcome.install_path = Some(install_path.to_string_lossy().to_string());
        outcome.quarantine_path = Some(quarantine.to_string_lossy().to_string());
        if outcome.status == UninstallOutcomeStatus::Finalized {
            outcome.status = UninstallOutcomeStatus::Removed;
        }
        outcome
    }

    /// Completes an uninstall whose content is already quarantined.
    fn finish_quarantine(
        &self,
        repository: &CapabilityRepository,
        recorded: &SkillInstallation,
        install_path: &Path,
    ) -> UninstallOutcome {
        let target_root = install_path.parent().unwrap_or(install_path);
        let prefix = format!(".{}.cs-quarantine-", recorded.skill_name);
        let quarantine = std::fs::read_dir(target_root).ok().and_then(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .find(|path| {
                    path.file_name()
                        .map(|name| name.to_string_lossy().starts_with(&prefix))
                        .unwrap_or(false)
                })
        });

        let Some(quarantine) = quarantine else {
            return UninstallOutcome {
                target_id: recorded.target_id.clone(),
                skill_name: recorded.skill_name.clone(),
                status: UninstallOutcomeStatus::NotFound,
                install_path: Some(install_path.to_string_lossy().to_string()),
                quarantine_path: None,
                detail: Some("no quarantine directory remains to finalize".to_string()),
            };
        };
        let _ = std::fs::remove_dir_all(&quarantine);
        let mut outcome = self.finalize_row(repository, recorded, Some(&quarantine));
        outcome.quarantine_path = Some(quarantine.to_string_lossy().to_string());
        outcome
    }

    fn finalize_row(
        &self,
        repository: &CapabilityRepository,
        recorded: &SkillInstallation,
        quarantine: Option<&Path>,
    ) -> UninstallOutcome {
        if let Some(quarantine) = quarantine {
            let _ = std::fs::remove_dir_all(quarantine);
        }
        match repository.set_installation_state(&recorded.installation_id, SkillInstallationState::Removed) {
            Ok(_) => UninstallOutcome {
                target_id: recorded.target_id.clone(),
                skill_name: recorded.skill_name.clone(),
                status: UninstallOutcomeStatus::Finalized,
                install_path: Some(recorded.install_path.clone()),
                quarantine_path: None,
                detail: None,
            },
            Err(error) => UninstallOutcome {
                target_id: recorded.target_id.clone(),
                skill_name: recorded.skill_name.clone(),
                status: UninstallOutcomeStatus::Refused,
                install_path: Some(recorded.install_path.clone()),
                quarantine_path: None,
                detail: Some(error.redacted_message()),
            },
        }
    }
}

/// A quarantine directory left behind by an interrupted uninstall.
pub fn is_quarantine_entry(name: &str) -> bool {
    name.starts_with('.') && name.contains(".cs-quarantine-")
}

/// The parent directory an install path lives in.
pub fn install_parent(install_path: &Path) -> PathBuf {
    install_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::skill::checker::check_directory;
    use crate::capability::skill::installer::SkillInstaller;
    use crate::capability::skill::plan::SkillInstallPlan;
    use crate::capability::skill::source::SkillSource;
    use crate::capability::types::{CapabilityKind, OperationBegin, OperationRequest};
    use crate::capability::repository::CapabilityRepository;
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

    fn operation(fixture: &Fixture, idempotency_key: &str) -> String {
        let request = OperationRequest {
            capability: CapabilityKind::Skill,
            operation_kind: "skill.uninstall".to_string(),
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

    /// Installs the fixture skill into the ChatSpeed target and returns its path.
    fn installed(fixture: &Fixture) -> PathBuf {
        let report = check_directory(&fixture.content).expect("check");
        let source = SkillSource::LocalDirectory {
            path: fixture.content.to_string_lossy().to_string(),
        };
        let plan = SkillInstallPlan::build(
            &source,
            &report,
            &fixture.content,
            &[crate::capability::targets::SkillTargetId::Chatspeed],
            fixture.frozen.clone(),
        )
        .expect("plan");
        let installer = SkillInstaller::new(fixture.environment.clone());
        let summary = installer
            .apply(&plan, &fixture.repository, &operation(fixture, "install-1"))
            .expect("apply");
        assert_eq!(
            summary.outcomes[0].status,
            crate::capability::skill::installer::TargetOutcomeStatus::Installed
        );
        PathBuf::from(summary.outcomes[0].install_path.clone().unwrap())
    }

    #[test]
    fn a_managed_untouched_installation_is_removed() {
        let fixture = fixture();
        let install_path = installed(&fixture);
        let uninstaller = SkillUninstaller::new(fixture.environment.clone());

        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-1"),
                "chatspeed",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Removed);
        assert!(!install_path.exists());
        let row = fixture
            .repository
            .get_installation("chatspeed", "demo")
            .expect("lookup")
            .expect("row");
        assert_eq!(row.state, SkillInstallationState::Removed);
    }

    #[test]
    fn an_unmanaged_directory_is_never_deleted() {
        let fixture = fixture();
        let skills_dir = fixture
            .environment
            .chatspeed_skills_dir()
            .expect("skills dir");
        std::fs::create_dir_all(skills_dir.join("demo")).expect("create dir");
        std::fs::write(skills_dir.join("demo/handwritten.md"), "mine").expect("write");

        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-1"),
                "chatspeed",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::NotFound);
        assert!(skills_dir.join("demo/handwritten.md").is_file());
    }

    #[test]
    fn drifted_content_is_refused_and_kept() {
        let fixture = fixture();
        let install_path = installed(&fixture);
        std::fs::write(install_path.join("notes.md"), "edited by hand").expect("edit");

        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-1"),
                "chatspeed",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Refused);
        assert!(install_path.join("notes.md").is_file());
        assert!(outcome
            .detail
            .as_deref()
            .expect("detail")
            .contains("drifted"));
    }

    #[test]
    fn a_removed_marker_is_refused_and_the_content_is_kept() {
        let fixture = fixture();
        let install_path = installed(&fixture);
        std::fs::remove_file(install_path.join(".chatspeed-skill.json")).expect("remove marker");

        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-1"),
                "chatspeed",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Refused);
        assert!(install_path.join("SKILL.md").is_file());
    }

    #[test]
    fn a_reserved_name_is_refused() {
        let fixture = fixture();
        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-1"),
                "chatspeed",
                "chatspeed-cli",
            )
            .expect("uninstall");
        assert_eq!(outcome.status, UninstallOutcomeStatus::Refused);
    }

    #[test]
    fn an_unsupported_target_is_refused_without_deleting_anything() {
        let fixture = fixture();
        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-1"),
                "cursor",
                "demo",
            )
            .expect("uninstall");
        assert_eq!(outcome.status, UninstallOutcomeStatus::Refused);
    }

    #[test]
    fn a_missing_directory_finalizes_the_row_instead_of_failing() {
        let fixture = fixture();
        let install_path = installed(&fixture);
        std::fs::remove_dir_all(&install_path).expect("remove by hand");

        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-1"),
                "chatspeed",
                "demo",
            )
            .expect("uninstall");
        assert_eq!(outcome.status, UninstallOutcomeStatus::Finalized);
        let row = fixture
            .repository
            .get_installation("chatspeed", "demo")
            .expect("lookup")
            .expect("row");
        assert_eq!(row.state, SkillInstallationState::Removed);
    }

    #[test]
    fn a_quarantine_left_behind_by_a_crash_is_finalized_without_re_deleting() {
        let fixture = fixture();
        let install_path = installed(&fixture);
        let target_root = install_path.parent().expect("parent").to_path_buf();
        let quarantine = target_root.join(".demo.cs-quarantine-crashed");
        std::fs::rename(&install_path, &quarantine).expect("simulate the interrupted rename");
        fixture
            .repository
            .set_installation_state(
                &fixture
                    .repository
                    .get_installation("chatspeed", "demo")
                    .expect("lookup")
                    .expect("row")
                    .installation_id,
                SkillInstallationState::Quarantined,
            )
            .expect("mark quarantined");

        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-1"),
                "chatspeed",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Finalized);
        assert!(!quarantine.exists());
    }
}
