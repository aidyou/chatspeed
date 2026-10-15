//! Explicit Agent Skill uninstallation.
//!
//! Deletion is only ever attempted for a `(target, name)` pair that passes
//! every gate below, and the path that is removed is always derived as
//! `registered_target_root.join(name)` — never a caller-supplied path
//! (AC-3/AC-4/INV-5):
//!
//! * the name is not reserved;
//! * the id is a registered target, and `target_preflight::verify_delete_root`
//!   proves its root and every existing ancestor is a real directory (never a
//!   symbolic link) and that the id canonically owns the directory it resolves
//!   to (AC-7/INV-6);
//! * the joined path sits directly under that root and is itself a real
//!   directory, never a symbolic link;
//! * for a managed directory a durable installation row exists for exactly
//!   this path, its ownership marker matches the row, and the content still
//!   matches the recorded manifest (no drift) — drifted user content is never
//!   deleted (AC-7/INV-6).
//!
//! A directory with no ownership row is a *discovered* Skill. It may be removed
//! only because the caller explicitly asked for this exact `(target, name)` —
//! the UI has already shown the user the concrete directory — and it is still
//! subject to the same registered-target, exact-path and real-directory gates.
//! It is deleted directly, because there is no durable row that could finalize a
//! quarantine; the intent and the observation are journaled by [`SkillUninstaller::uninstall`].
//!
//! A managed directory is first renamed to a sibling quarantine inside the same
//! target, then the row is finalized and the quarantine is removed, so an
//! interrupted uninstall is always recognizable rather than half-deleted.
//!
//! Anything else is refused with zero deletion (AC-7/INV-6).

use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::capability::error::CapabilityError;
use crate::capability::repository::CapabilityRepository;
use crate::capability::skill::manifest::{verify_file_manifest, ManifestVerdict};
use crate::capability::skill::ownership;
use crate::capability::skill_inventory::RESERVED_SKILL_NAMES;
use crate::capability::target_preflight;
use crate::capability::targets::{SkillTargetId, TargetEnvironment};
use crate::capability::types::{EffectOutcome, SkillInstallation, SkillInstallationState};

/// Whether `target_id` is a registered Agent Skill target ChatSpeed may delete
/// from.
///
/// Every registered target is a candidate, including an external tool's target
/// and one that still has no ChatSpeed ownership row. The id alone is never
/// authority to delete: `target_preflight::verify_delete_root` makes the
/// physical decision, refusing an unsupported target, a directory chain that
/// contains a symbolic link, and an id that only aliases a higher-priority
/// target's directory. An unregistered id is never eligible.
pub fn is_uninstall_eligible_target(target_id: &str) -> bool {
    SkillTargetId::parse(target_id).is_some()
}

/// The physical kind of the exact skill path a delete would touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkillPathKind {
    /// Nothing exists at the path.
    Missing,
    /// A real directory: the only kind ChatSpeed will ever delete.
    RealDirectory,
    /// A symbolic link, a regular file, or something that cannot be inspected.
    Unsafe,
}

/// Classifies the exact skill path without following a final symbolic link, so
/// a linked directory can never be deleted through.
fn classify_skill_path(path: &Path) -> SkillPathKind {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => SkillPathKind::Unsafe,
        Ok(metadata) if metadata.is_dir() => SkillPathKind::RealDirectory,
        Ok(_) => SkillPathKind::Unsafe,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => SkillPathKind::Missing,
        Err(_) => SkillPathKind::Unsafe,
    }
}

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
            UninstallOutcomeStatus::Removed | UninstallOutcomeStatus::Finalized => {
                EffectOutcome::Applied
            }
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
        // Convergence follows the same delete eligibility as an explicit uninstall:
        // any registered target is a candidate, and only durable proof (marker +
        // manifest) is ever acted on, so an unproven or drifted external
        // directory is still preserved untouched.
        if !is_uninstall_eligible_target(target_id) {
            return Ok(None);
        }
        // A target that only aliases a higher-priority target's physical
        // directory — for example `chatspeed` when `CHATSPEED_HOME` points at
        // the shared Agents home — is not its owner, so a legacy ownership row
        // or quarantine must never be converged through it (AC-7/INV-2).
        if !target_preflight::is_canonical_owner(target_id, &self.environment) {
            return Ok(None);
        }
        let Some(recorded) = repository.get_installation(target_id, skill_name)? else {
            return Ok(None);
        };
        let install_path = PathBuf::from(&recorded.install_path);
        match recorded.state {
            // A quarantine that never finalized: delete only the directory
            // ChatSpeed moved aside and record the row as removed.
            SkillInstallationState::Quarantined => {
                let outcome = self.finish_quarantine(repository, &recorded, &install_path);
                if outcome.status == UninstallOutcomeStatus::Finalized {
                    Ok(Some(outcome))
                } else {
                    Ok(None)
                }
            }
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

        // Only a registered target is ever touched: an unregistered id has no
        // verified directory and is refused before anything is resolved.
        if !is_uninstall_eligible_target(target_id) {
            return self.refuse(
                target_id,
                skill_name,
                None,
                "this target is not a registered Agent Skill target",
            );
        }

        // Before any deletion the eligible root must be a real directory: a
        // symlinked target or ancestor is refused so an eligible id can never
        // delete through a link that points at an external software root
        // (AC-7/INV-6).
        let target_root = match target_preflight::verify_delete_root(target_id, &self.environment) {
            Ok(verified) => verified.root,
            Err(failure) => {
                return self.refuse(target_id, skill_name, None, failure.detail());
            }
        };
        // The delete is bound to exactly one path: `root.join(name)`. A name
        // that would resolve anywhere but directly under the verified root (for
        // example one containing a path separator) is refused, so no request can
        // reach an arbitrary directory (AC-3/AC-4/INV-5).
        let install_path = target_root.join(skill_name);
        if install_path.parent() != Some(target_root.as_path()) {
            return self.refuse(
                target_id,
                skill_name,
                None,
                "the skill name does not resolve to a directory directly under the target root",
            );
        }
        let path_kind = classify_skill_path(&install_path);

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
            // No ownership row: a directory here is a discovered Skill the user
            // explicitly selected for this exact `(target, name)`. There is no
            // durable proof or quarantine to reconcile, so it is removed
            // directly under the same target/path/real-directory gates.
            return self.uninstall_discovered(target_id, skill_name, &install_path, path_kind);
        };

        if recorded.install_path != install_path.to_string_lossy() {
            return self.refuse(
                target_id,
                skill_name,
                Some(&install_path),
                "the recorded installation belongs to a different path",
            );
        }

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
                    format!(
                        "the installation is in state '{}' and is not uninstallable",
                        other.as_str()
                    ),
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
        match path_kind {
            // The directory is already gone; finalize the row instead of
            // guessing about a deletion that already happened.
            SkillPathKind::Missing => return self.finalize_row(repository, &recorded),
            // A linked or non-directory path is never deleted through: the only
            // deletion authority is a real directory at the exact target path.
            SkillPathKind::Unsafe => {
                return self.refuse(
                    target_id,
                    skill_name,
                    Some(&install_path),
                    "the installed path is a symbolic link or is not a directory",
                );
            }
            SkillPathKind::RealDirectory => {}
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
        let quarantine = target_root.join(quarantine_name(&recorded));
        match std::fs::symlink_metadata(&quarantine) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => {
                return self.refuse(
                    target_id,
                    skill_name,
                    Some(&install_path),
                    "the quarantine path already exists or cannot be inspected",
                );
            }
        }
        if let Err(error) = std::fs::rename(&install_path, &quarantine) {
            return self.refuse(
                target_id,
                skill_name,
                Some(&install_path),
                format!("failed to quarantine the installed skill: {error}"),
            );
        }

        match repository.set_installation_state(
            &recorded.installation_id,
            SkillInstallationState::Quarantined,
        ) {
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

        let mut outcome = self.finish_quarantine(repository, &recorded, &install_path);
        outcome.install_path = Some(install_path.to_string_lossy().to_string());
        outcome.quarantine_path = Some(quarantine.to_string_lossy().to_string());
        if outcome.status == UninstallOutcomeStatus::Finalized {
            outcome.status = UninstallOutcomeStatus::Removed;
        }
        outcome
    }

    /// Removes a discovered (unowned) Skill directory the caller explicitly
    /// selected.
    ///
    /// No ownership row exists, so there is no marker or manifest proof to
    /// check. The authority is only the caller's explicit request for this exact
    /// `(target, name)`, which reached here only after a registered target
    /// passed `verify_delete_root` and the joined path was proven to sit
    /// directly under that root. A symbolic link or a non-directory is refused
    /// rather than followed, so a discovered entry can never delete through a
    /// link that leaves the verified target. There is no durable row that could
    /// finalize a quarantine, so the directory is removed directly; the intent
    /// and the observation are journaled by [`SkillUninstaller::uninstall`].
    fn uninstall_discovered(
        &self,
        target_id: &str,
        skill_name: &str,
        install_path: &Path,
        path_kind: SkillPathKind,
    ) -> UninstallOutcome {
        match path_kind {
            SkillPathKind::Missing => UninstallOutcome {
                target_id: target_id.to_string(),
                skill_name: skill_name.to_string(),
                status: UninstallOutcomeStatus::NotFound,
                install_path: Some(install_path.to_string_lossy().to_string()),
                quarantine_path: None,
                detail: Some(
                    "no ownership record and no directory exist for this name".to_string(),
                ),
            },
            SkillPathKind::Unsafe => self.refuse(
                target_id,
                skill_name,
                Some(install_path),
                "the discovered path is a symbolic link or is not a directory",
            ),
            SkillPathKind::RealDirectory => match std::fs::remove_dir_all(install_path) {
                Ok(()) => UninstallOutcome {
                    target_id: target_id.to_string(),
                    skill_name: skill_name.to_string(),
                    status: UninstallOutcomeStatus::Removed,
                    install_path: Some(install_path.to_string_lossy().to_string()),
                    quarantine_path: None,
                    detail: Some(
                        "the explicitly selected discovered directory was removed".to_string(),
                    ),
                },
                Err(error) => self.refuse(
                    target_id,
                    skill_name,
                    Some(install_path),
                    format!("failed to remove the discovered skill directory: {error}"),
                ),
            },
        }
    }

    /// Completes an uninstall whose content is already quarantined.
    ///
    /// The quarantine location is deterministic from the durable installation
    /// id. We never scan by skill-name prefix: an unproven sibling (including a
    /// symlink or a second candidate) is user content and must remain intact.
    fn finish_quarantine(
        &self,
        repository: &CapabilityRepository,
        recorded: &SkillInstallation,
        install_path: &Path,
    ) -> UninstallOutcome {
        if !is_uninstall_eligible_target(&recorded.target_id) {
            return self.refuse(
                &recorded.target_id,
                &recorded.skill_name,
                Some(install_path),
                "this target is not eligible for quarantine deletion",
            );
        }
        let target_root =
            match target_preflight::verify_delete_root(&recorded.target_id, &self.environment) {
                Ok(verified) => verified.root,
                Err(failure) => {
                    return self.refuse(
                        &recorded.target_id,
                        &recorded.skill_name,
                        Some(install_path),
                        failure.detail(),
                    );
                }
            };
        let expected_install = target_root.join(&recorded.skill_name);
        if install_path != expected_install
            || recorded.install_path != expected_install.to_string_lossy()
        {
            return self.refuse(
                &recorded.target_id,
                &recorded.skill_name,
                Some(install_path),
                "the quarantine ownership row does not match the fixed target path",
            );
        }
        let quarantine = target_root.join(quarantine_name(recorded));
        let Some(verified) = prove_quarantine(&quarantine, recorded) else {
            return self.refuse(
                &recorded.target_id,
                &recorded.skill_name,
                Some(install_path),
                "the durable quarantine path is missing or its ownership/content proof is invalid",
            );
        };

        if let Err(error) = std::fs::remove_dir_all(&verified) {
            return self.refuse(
                &recorded.target_id,
                &recorded.skill_name,
                Some(install_path),
                format!("failed to remove the proven quarantine: {error}"),
            );
        }
        let mut outcome = self.finalize_row(repository, recorded);
        outcome.quarantine_path = Some(quarantine.to_string_lossy().to_string());
        outcome
    }

    fn finalize_row(
        &self,
        repository: &CapabilityRepository,
        recorded: &SkillInstallation,
    ) -> UninstallOutcome {
        match repository
            .set_installation_state(&recorded.installation_id, SkillInstallationState::Removed)
        {
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

/// Derives the only quarantine name that can belong to a durable installation.
fn quarantine_name(recorded: &SkillInstallation) -> String {
    let mut hasher = Sha256::new();
    hasher.update(recorded.installation_id.as_bytes());
    let identity = hex::encode(hasher.finalize());
    format!(".{}.cs-quarantine-{identity}", recorded.skill_name)
}

/// Re-proves a quarantine before deletion. It must be the exact deterministic
/// sibling, a real directory, and contain the same ownership marker and content
/// manifest as the durable installation row. `None` means preserve it.
fn prove_quarantine(path: &Path, recorded: &SkillInstallation) -> Option<PathBuf> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return None;
    }
    if path.file_name()? != quarantine_name(recorded).as_str() {
        return None;
    }
    let prefix = format!(".{}.cs-quarantine-", recorded.skill_name);
    let parent = path.parent()?;
    let entries = std::fs::read_dir(parent).ok()?;
    for entry in entries {
        let entry = entry.ok()?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(&prefix) && name != quarantine_name(recorded).as_str()
        {
            // An ambiguous set of quarantine candidates cannot be attributed
            // to this installation. Preserve every candidate and wait for
            // explicit reconciliation evidence rather than deleting one.
            return None;
        }
    }
    let digest = crate::capability::skill::manifest::manifest_digest(&recorded.file_manifest)?;
    if digest != recorded.manifest_digest || digest != recorded.content_digest {
        return None;
    }
    if !ownership::has_proof(path, recorded).ok()? {
        return None;
    }
    if !verify_file_manifest(path, &recorded.file_manifest)
        .ok()?
        .is_match()
    {
        return None;
    }
    Some(path.to_path_buf())
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
    use crate::capability::repository::CapabilityRepository;
    use crate::capability::skill::checker::check_directory;
    use crate::capability::skill::installer::SkillInstaller;
    use crate::capability::skill::plan::SkillInstallPlan;
    use crate::capability::skill::source::SkillSource;
    use crate::capability::types::{CapabilityKind, OperationBegin, OperationRequest};
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
        let chatspeed = temp.path().join("chatspeed-home");
        fixture_in(temp, chatspeed)
    }

    /// A fixture whose `CHATSPEED_HOME` points at the shared Agents home, so the
    /// `chatspeed` and `agents` ids resolve to one physical directory.
    fn aliased_fixture() -> Fixture {
        let temp = TempDir::new().expect("temp dir");
        let chatspeed = temp.path().join("home").join(".agents");
        fixture_in(temp, chatspeed)
    }

    fn fixture_in(temp: TempDir, chatspeed: PathBuf) -> Fixture {
        let home = temp.path().join("home");
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

    /// Plants a managed skill directory under the shared Agents home and records
    /// an ownership row for `target_id`, so the directory is otherwise fully
    /// deletable and only the target's right to delete can refuse it.
    fn plant_managed_skill(fixture: &Fixture, target_id: &str, name: &str) -> PathBuf {
        let directory = fixture
            .environment
            .home_dir
            .clone()
            .expect("home")
            .join(".agents")
            .join("skills")
            .join(name);
        std::fs::create_dir_all(&directory).expect("create skill dir");
        std::fs::write(
            directory.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: test skill\n---\n\n# {name}\n"),
        )
        .expect("write skill");
        let manifest = crate::capability::skill::manifest::compute_file_manifest(&directory)
            .expect("manifest");
        let manifest_digest = crate::capability::skill::manifest::manifest_digest(&manifest)
            .expect("manifest digest");
        let content_digest = crate::capability::operation::content_digest(
            &manifest
                .iter()
                .map(|entry| (entry.path.clone(), entry.sha256.clone()))
                .collect::<Vec<_>>(),
        );
        let installation = SkillInstallation {
            installation_id: format!("skl-{target_id}-{name}"),
            skill_name: name.to_string(),
            target_id: target_id.to_string(),
            install_path: directory.to_string_lossy().to_string(),
            source_kind: "local_directory".to_string(),
            source_ref: "local_directory:test".to_string(),
            checker_version: "skill-checker.v1".to_string(),
            verdict: "pass".to_string(),
            content_digest,
            file_manifest: manifest,
            marker_nonce: format!("nonce-{target_id}-{name}"),
            manifest_digest,
            state: SkillInstallationState::Installed,
            operation_id: None,
            created_at_ms: 0,
            updated_at_ms: 0,
        };
        crate::capability::skill::ownership::write_marker(
            &directory,
            &crate::capability::skill::ownership::OwnershipMarker::from_installation(&installation),
        )
        .expect("write marker");
        fixture
            .repository
            .upsert_installation(&installation)
            .expect("record ownership");
        directory
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
        installed_into(fixture, SkillTargetId::Chatspeed)
    }

    /// Installs the fixture skill into one target and returns its install path.
    fn installed_into(fixture: &Fixture, target: SkillTargetId) -> PathBuf {
        let report = check_directory(&fixture.content).expect("check");
        let source = SkillSource::LocalDirectory {
            path: fixture.content.to_string_lossy().to_string(),
        };
        let plan = SkillInstallPlan::build(
            &source,
            &report,
            &fixture.content,
            &[target],
            fixture.frozen.join(target.as_str()),
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
    fn a_discovered_directory_is_removed_when_explicitly_selected() {
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

        assert_eq!(outcome.status, UninstallOutcomeStatus::Removed);
        assert!(!skills_dir.join("demo").exists());
        assert!(
            fixture
                .repository
                .get_installation("chatspeed", "demo")
                .expect("lookup")
                .is_none(),
            "no ownership row is invented for a discovered directory"
        );
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
    fn an_unregistered_target_is_refused_and_deletes_nothing() {
        let fixture = fixture();
        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-unknown-target"),
                "not-a-registered-target",
                "demo",
            )
            .expect("uninstall");
        assert_eq!(outcome.status, UninstallOutcomeStatus::Refused);
    }

    #[test]
    fn an_external_target_without_a_directory_is_not_found() {
        let fixture = fixture();
        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-1"),
                "codex",
                "demo",
            )
            .expect("uninstall");
        assert_eq!(outcome.status, UninstallOutcomeStatus::NotFound);
    }

    #[test]
    fn an_external_discovered_directory_is_removed_when_explicitly_selected() {
        let fixture = fixture();
        let codex_root = fixture
            .environment
            .home_dir
            .clone()
            .expect("home")
            .join(".codex")
            .join("skills");
        let directory = codex_root.join("demo");
        std::fs::create_dir_all(&directory).expect("create discovered directory");
        std::fs::write(directory.join("SKILL.md"), "---\nname: demo\n---\n").expect("write");

        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-external-discovered"),
                "codex",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Removed);
        assert!(!directory.exists());
        assert!(
            fixture
                .repository
                .get_installation("codex", "demo")
                .expect("lookup")
                .is_none(),
            "no ownership row is invented for a discovered external directory"
        );
    }

    #[test]
    fn a_name_that_escapes_the_target_root_is_refused() {
        let fixture = fixture();
        let chatspeed_home = fixture.environment.chatspeed_home.clone().expect("home");
        // A sibling of the target root with the escaped name must survive: the
        // delete is bound to exactly `root.join(name)`.
        let escaped = chatspeed_home.join("evil");
        std::fs::create_dir_all(&escaped).expect("create escaped dir");
        std::fs::write(escaped.join("keep.md"), "keep me").expect("write");

        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-escape"),
                "chatspeed",
                "../evil",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Refused);
        assert!(escaped.join("keep.md").is_file());
    }

    #[test]
    fn an_external_install_is_removed_with_proof() {
        let fixture = fixture();
        let install_path = installed_into(&fixture, SkillTargetId::Codex);
        let target_root = install_path.parent().expect("parent").to_path_buf();

        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-1"),
                "codex",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Removed);
        assert!(!install_path.exists());
        assert!(
            !std::fs::read_dir(&target_root)
                .expect("read target root")
                .flatten()
                .any(|entry| is_quarantine_entry(&entry.file_name().to_string_lossy())),
            "the proven quarantine is removed, not left behind"
        );
        let row = fixture
            .repository
            .get_installation("codex", "demo")
            .expect("lookup")
            .expect("row");
        assert_eq!(row.state, SkillInstallationState::Removed);
    }

    #[test]
    fn an_external_drifted_install_is_refused_and_kept() {
        let fixture = fixture();
        let install_path = installed_into(&fixture, SkillTargetId::Codex);
        std::fs::write(install_path.join("notes.md"), "edited by hand").expect("edit");

        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-external-drift"),
                "codex",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Refused);
        assert!(install_path.join("notes.md").is_file());
        assert_eq!(
            fixture
                .repository
                .get_installation("codex", "demo")
                .expect("lookup")
                .expect("row")
                .state,
            SkillInstallationState::Installed,
            "a drifted external install is left exactly as it was"
        );
    }

    #[test]
    fn a_canonical_agents_install_is_removed_with_proof() {
        let fixture = fixture();
        let install_path = installed_into(&fixture, SkillTargetId::Agents);

        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-1"),
                "agents",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Removed);
        assert!(!install_path.exists());
        let row = fixture
            .repository
            .get_installation("agents", "demo")
            .expect("lookup")
            .expect("row");
        assert_eq!(row.state, SkillInstallationState::Removed);
    }

    #[test]
    fn a_canonical_agents_install_is_refused_when_drifted() {
        let fixture = fixture();
        let install_path = installed_into(&fixture, SkillTargetId::Agents);
        std::fs::write(install_path.join("notes.md"), "edited by hand").expect("edit");

        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-1"),
                "agents",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Refused);
        assert!(install_path.join("notes.md").is_file());
    }

    #[test]
    fn a_legacy_chatspeed_row_cannot_delete_the_canonical_agents_directory() {
        let fixture = aliased_fixture();
        let directory = plant_managed_skill(&fixture, "chatspeed", "demo");

        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-legacy-alias"),
                "chatspeed",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Refused);
        assert!(
            directory.join("SKILL.md").is_file(),
            "the canonical Agents content is retained"
        );
        assert_eq!(
            fixture
                .repository
                .get_installation("chatspeed", "demo")
                .expect("lookup")
                .expect("row")
                .state,
            SkillInstallationState::Installed,
            "an aliasing row is left exactly as it was"
        );
    }

    #[test]
    fn a_canonical_agents_row_is_removed_in_an_aliased_environment() {
        let fixture = aliased_fixture();
        let directory = plant_managed_skill(&fixture, "agents", "demo");

        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-agents-alias"),
                "agents",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Removed);
        assert!(!directory.exists());
        assert_eq!(
            fixture
                .repository
                .get_installation("agents", "demo")
                .expect("lookup")
                .expect("row")
                .state,
            SkillInstallationState::Removed
        );
    }

    #[test]
    fn reconcile_recovers_a_proven_external_install() {
        let fixture = fixture();
        let install_path = installed_into(&fixture, SkillTargetId::Codex);
        let installation_id = fixture
            .repository
            .get_installation("codex", "demo")
            .expect("lookup")
            .expect("row")
            .installation_id;
        fixture
            .repository
            .set_installation_state(&installation_id, SkillInstallationState::Installing)
            .expect("mark installing");

        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let converged = uninstaller
            .reconcile_owned_directory(&fixture.repository, "codex", "demo")
            .expect("reconcile");

        assert_eq!(
            converged.map(|outcome| outcome.status),
            Some(UninstallOutcomeStatus::Finalized)
        );
        assert!(install_path.join("SKILL.md").is_file());
        assert_eq!(
            fixture
                .repository
                .get_installation("codex", "demo")
                .expect("lookup")
                .expect("row")
                .state,
            SkillInstallationState::Installed
        );
    }

    #[test]
    fn reconcile_leaves_a_drifted_external_install_untouched() {
        let fixture = fixture();
        let install_path = installed_into(&fixture, SkillTargetId::Codex);
        let installation_id = fixture
            .repository
            .get_installation("codex", "demo")
            .expect("lookup")
            .expect("row")
            .installation_id;
        fixture
            .repository
            .set_installation_state(&installation_id, SkillInstallationState::Installing)
            .expect("mark installing");
        std::fs::write(install_path.join("notes.md"), "edited by hand").expect("edit");

        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let converged = uninstaller
            .reconcile_owned_directory(&fixture.repository, "codex", "demo")
            .expect("reconcile");

        assert!(converged.is_none());
        assert!(install_path.join("notes.md").is_file());
        assert_eq!(
            fixture
                .repository
                .get_installation("codex", "demo")
                .expect("lookup")
                .expect("row")
                .state,
            SkillInstallationState::Installing,
            "drifted external content is never converged"
        );
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
        let recorded = fixture
            .repository
            .get_installation("chatspeed", "demo")
            .expect("lookup")
            .expect("row");
        let quarantine = target_root.join(quarantine_name(&recorded));
        std::fs::rename(&install_path, &quarantine).expect("simulate the interrupted rename");
        fixture
            .repository
            .set_installation_state(
                &recorded.installation_id,
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

    #[test]
    fn unproven_quarantine_candidates_are_preserved() {
        let fixture = fixture();
        let install_path = installed(&fixture);
        let target_root = install_path.parent().expect("parent").to_path_buf();
        let recorded = fixture
            .repository
            .get_installation("chatspeed", "demo")
            .expect("lookup")
            .expect("row");
        let fake = target_root.join(".demo.cs-quarantine-fake");
        std::fs::create_dir_all(&fake).expect("fake quarantine");
        std::fs::write(fake.join("user.txt"), "keep me").expect("fake content");
        fixture
            .repository
            .set_installation_state(
                &recorded.installation_id,
                SkillInstallationState::Quarantined,
            )
            .expect("mark quarantined");

        let outcome = SkillUninstaller::new(fixture.environment.clone())
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-unproven-quarantine"),
                "chatspeed",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Refused);
        assert!(fake.join("user.txt").is_file());
        assert_eq!(
            fixture
                .repository
                .get_installation("chatspeed", "demo")
                .expect("lookup")
                .expect("row")
                .state,
            SkillInstallationState::Quarantined
        );
    }

    #[test]
    fn drifted_quarantine_is_preserved() {
        let fixture = fixture();
        let install_path = installed(&fixture);
        let target_root = install_path.parent().expect("parent").to_path_buf();
        let recorded = fixture
            .repository
            .get_installation("chatspeed", "demo")
            .expect("lookup")
            .expect("row");
        let quarantine = target_root.join(quarantine_name(&recorded));
        std::fs::rename(&install_path, &quarantine).expect("quarantine");
        std::fs::write(quarantine.join("notes.md"), "drifted").expect("drift");
        fixture
            .repository
            .set_installation_state(
                &recorded.installation_id,
                SkillInstallationState::Quarantined,
            )
            .expect("mark quarantined");

        let outcome = SkillUninstaller::new(fixture.environment.clone())
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-drifted-quarantine"),
                "chatspeed",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Refused);
        assert!(quarantine.join("notes.md").is_file());
    }

    #[test]
    fn quarantine_with_a_wrong_marker_is_preserved() {
        let fixture = fixture();
        let install_path = installed(&fixture);
        let target_root = install_path.parent().expect("parent").to_path_buf();
        let recorded = fixture
            .repository
            .get_installation("chatspeed", "demo")
            .expect("lookup")
            .expect("row");
        let quarantine = target_root.join(quarantine_name(&recorded));
        std::fs::rename(&install_path, &quarantine).expect("quarantine");
        std::fs::write(
            quarantine.join(".chatspeed-skill.json"),
            b"{\"schema_version\":1,\"installation_id\":\"wrong\"}",
        )
        .expect("write wrong marker");
        fixture
            .repository
            .set_installation_state(
                &recorded.installation_id,
                SkillInstallationState::Quarantined,
            )
            .expect("mark quarantined");

        let outcome = SkillUninstaller::new(fixture.environment.clone())
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-wrong-marker"),
                "chatspeed",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Refused);
        assert!(quarantine.exists());
        assert_eq!(
            fixture
                .repository
                .get_installation("chatspeed", "demo")
                .expect("lookup")
                .expect("row")
                .state,
            SkillInstallationState::Quarantined
        );
    }

    #[test]
    fn quarantine_with_a_wrong_recorded_digest_is_preserved() {
        let fixture = fixture();
        let install_path = installed(&fixture);
        let target_root = install_path.parent().expect("parent").to_path_buf();
        let mut recorded = fixture
            .repository
            .get_installation("chatspeed", "demo")
            .expect("lookup")
            .expect("row");
        let quarantine = target_root.join(quarantine_name(&recorded));
        std::fs::rename(&install_path, &quarantine).expect("quarantine");
        recorded.content_digest = "wrong-content-digest".to_string();
        fixture
            .repository
            .upsert_installation(&recorded)
            .expect("update digest");
        fixture
            .repository
            .set_installation_state(
                &recorded.installation_id,
                SkillInstallationState::Quarantined,
            )
            .expect("mark quarantined");

        let outcome = SkillUninstaller::new(fixture.environment.clone())
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-wrong-digest"),
                "chatspeed",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Refused);
        assert!(quarantine.join("SKILL.md").is_file());
        assert_eq!(
            fixture
                .repository
                .get_installation("chatspeed", "demo")
                .expect("lookup")
                .expect("row")
                .state,
            SkillInstallationState::Quarantined
        );
    }

    #[test]
    fn a_non_directory_quarantine_is_preserved() {
        let fixture = fixture();
        let install_path = installed(&fixture);
        let target_root = install_path.parent().expect("parent").to_path_buf();
        let recorded = fixture
            .repository
            .get_installation("chatspeed", "demo")
            .expect("lookup")
            .expect("row");
        let quarantine = target_root.join(quarantine_name(&recorded));
        std::fs::remove_dir_all(&install_path).expect("remove install");
        std::fs::write(&quarantine, "not a directory").expect("write quarantine file");
        fixture
            .repository
            .set_installation_state(
                &recorded.installation_id,
                SkillInstallationState::Quarantined,
            )
            .expect("mark quarantined");

        let outcome = SkillUninstaller::new(fixture.environment.clone())
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-non-directory-quarantine"),
                "chatspeed",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Refused);
        assert!(quarantine.is_file());
    }

    #[test]
    fn multiple_quarantine_candidates_are_all_preserved() {
        let fixture = fixture();
        let install_path = installed(&fixture);
        let target_root = install_path.parent().expect("parent").to_path_buf();
        let recorded = fixture
            .repository
            .get_installation("chatspeed", "demo")
            .expect("lookup")
            .expect("row");
        let quarantine = target_root.join(quarantine_name(&recorded));
        let extra = target_root.join(".demo.cs-quarantine-extra");
        std::fs::rename(&install_path, &quarantine).expect("quarantine");
        std::fs::create_dir_all(&extra).expect("create extra candidate");
        std::fs::write(extra.join("user.txt"), "keep me").expect("write extra content");
        fixture
            .repository
            .set_installation_state(
                &recorded.installation_id,
                SkillInstallationState::Quarantined,
            )
            .expect("mark quarantined");

        let outcome = SkillUninstaller::new(fixture.environment.clone())
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-multiple-quarantines"),
                "chatspeed",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Refused);
        assert!(quarantine.join("SKILL.md").is_file());
        assert!(extra.join("user.txt").is_file());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_quarantine_is_preserved() {
        let fixture = fixture();
        let install_path = installed(&fixture);
        let target_root = install_path.parent().expect("parent").to_path_buf();
        let recorded = fixture
            .repository
            .get_installation("chatspeed", "demo")
            .expect("lookup")
            .expect("row");
        let outside = fixture._temp.path().join("quarantine-outside");
        std::fs::create_dir_all(&outside).expect("outside");
        std::fs::write(outside.join("user.txt"), "keep me").expect("outside content");
        let quarantine = target_root.join(quarantine_name(&recorded));
        std::os::unix::fs::symlink(&outside, &quarantine).expect("symlink quarantine");
        std::fs::remove_dir_all(&install_path).expect("remove install");
        fixture
            .repository
            .set_installation_state(
                &recorded.installation_id,
                SkillInstallationState::Quarantined,
            )
            .expect("mark quarantined");

        let outcome = SkillUninstaller::new(fixture.environment.clone())
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-symlink-quarantine"),
                "chatspeed",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Refused);
        assert!(quarantine.exists());
        assert!(outside.join("user.txt").is_file());
    }
    #[test]
    fn a_symlinked_eligible_root_is_refused_and_the_content_is_kept() {
        #[cfg(unix)]
        {
            let fixture = fixture();
            let install_path = installed(&fixture);
            let skills = fixture
                .environment
                .chatspeed_skills_dir()
                .expect("skills dir");

            // Replace the eligible root with a link that points at an external
            // directory holding the installed skill.
            let outside = fixture._temp.path().join("outside");
            std::fs::create_dir_all(&outside).expect("create outside");
            std::fs::rename(&skills, outside.join("skills")).expect("move skills aside");
            std::os::unix::fs::symlink(outside.join("skills"), &skills).expect("symlink root");

            let uninstaller = SkillUninstaller::new(fixture.environment.clone());
            let outcome = uninstaller
                .uninstall(
                    &fixture.repository,
                    &operation(&fixture, "uninstall-symlink"),
                    "chatspeed",
                    "demo",
                )
                .expect("uninstall");

            assert_eq!(outcome.status, UninstallOutcomeStatus::Refused);
            assert!(
                outside.join("skills/demo/SKILL.md").is_file(),
                "the external content is retained"
            );
            assert!(
                install_path.exists(),
                "the eligible root link still resolves to the retained content"
            );
            let row = fixture
                .repository
                .get_installation("chatspeed", "demo")
                .expect("lookup")
                .expect("row");
            assert_eq!(row.state, SkillInstallationState::Installed);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_discovered_skill_directory_is_refused_and_the_target_is_kept() {
        let fixture = fixture();
        let codex_root = fixture
            .environment
            .home_dir
            .clone()
            .expect("home")
            .join(".codex")
            .join("skills");
        std::fs::create_dir_all(&codex_root).expect("create codex skills dir");
        let outside = fixture._temp.path().join("discovered-outside");
        std::fs::create_dir_all(&outside).expect("create outside");
        std::fs::write(outside.join("keep.md"), "keep me").expect("write outside content");
        let link = codex_root.join("demo");
        std::os::unix::fs::symlink(&outside, &link).expect("symlink skill dir");

        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-symlink-discovered"),
                "codex",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Refused);
        assert!(outside.join("keep.md").is_file());
        assert!(
            std::fs::symlink_metadata(&link)
                .expect("link metadata")
                .file_type()
                .is_symlink(),
            "the link itself is preserved"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_managed_skill_directory_is_refused_and_the_content_is_kept() {
        let fixture = fixture();
        let install_path = installed(&fixture);
        let outside = fixture._temp.path().join("managed-outside");
        std::fs::rename(&install_path, &outside).expect("move content aside");
        std::os::unix::fs::symlink(&outside, &install_path).expect("symlink skill dir");

        let uninstaller = SkillUninstaller::new(fixture.environment.clone());
        let outcome = uninstaller
            .uninstall(
                &fixture.repository,
                &operation(&fixture, "uninstall-symlink-managed"),
                "chatspeed",
                "demo",
            )
            .expect("uninstall");

        assert_eq!(outcome.status, UninstallOutcomeStatus::Refused);
        assert!(outside.join("SKILL.md").is_file());
        assert_eq!(
            fixture
                .repository
                .get_installation("chatspeed", "demo")
                .expect("lookup")
                .expect("row")
                .state,
            SkillInstallationState::Installed
        );
    }
}
