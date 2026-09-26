//! Immutable Agent Skill install plans.
//!
//! A plan freezes everything an install needs *before* any target is touched:
//! the checked content, its manifest, the verdict that authorized it and the
//! exact target selection. The freeze is a private copy inside the operation's
//! staging directory, so a source that changes between "check" and "install"
//! cannot smuggle different bytes in, and every file is re-hashed while it is
//! copied (AC-5).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::capability::error::CapabilityError;
use crate::capability::operation;
use crate::capability::skill::checker::{SkillCheckReport, SKILL_CHECKER_VERSION};
use crate::capability::skill::manifest::{compute_file_manifest, manifest_digest};
use crate::capability::skill::source::SkillSource;
use crate::capability::targets::SkillTargetId;
use crate::capability::types::SkillFileEntry;

/// Plan layout version.
pub const PLAN_SCHEMA_VERSION: u32 = 1;

/// A frozen, already-authorized install plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInstallPlan {
    pub plan_id: String,
    pub schema_version: u32,
    pub skill_name: String,
    pub source_kind: String,
    pub source_ref: String,
    pub checker_version: String,
    pub verdict: String,
    pub content_digest: String,
    pub files: Vec<SkillFileEntry>,
    /// The frozen target selection. Selection is never re-derived at apply
    /// time, so a target cannot be added by a later step.
    pub targets: Vec<String>,
    /// Private copy of the checked content, inside the operation's staging.
    pub frozen_root: PathBuf,
}

impl SkillInstallPlan {
    /// Freezes `content_root` and builds the plan.
    ///
    /// A plan can only be built from a `pass` verdict produced by the current
    /// checker version; anything else is refused, so no later step can install
    /// content the checker did not authorize (INV-4).
    pub fn build(
        source: &SkillSource,
        report: &SkillCheckReport,
        content_root: &Path,
        targets: &[SkillTargetId],
        frozen_root: PathBuf,
    ) -> Result<Self, CapabilityError> {
        report.validate()?;
        if !report.is_pass() {
            return Err(report.refusal().unwrap_or_else(|| {
                CapabilityError::refused("the skill did not pass the checker")
            }));
        }
        if report.checker_version != SKILL_CHECKER_VERSION {
            return Err(CapabilityError::refused(
                "the check was produced by a different checker version",
            ));
        }
        if targets.is_empty() {
            return Err(CapabilityError::invalid_request(
                "an install plan requires at least one target",
            ));
        }
        let mut unique: Vec<String> = targets.iter().map(|target| target.as_str().to_string()).collect();
        unique.sort();
        unique.dedup();
        if unique.len() != targets.len() {
            return Err(CapabilityError::invalid_request(
                "an install plan must not select the same target twice",
            ));
        }

        let skill_name = report.skill_name.clone().ok_or_else(|| {
            CapabilityError::refused("the checked skill has no declared name")
        })?;
        let expected_digest = report.content_digest.clone().ok_or_else(|| {
            CapabilityError::refused("the checked skill has no content digest")
        })?;

        freeze_content(content_root, &frozen_root)?;
        let frozen = compute_file_manifest(&frozen_root)?;
        let frozen_digest = manifest_digest(&frozen).ok_or_else(|| {
            CapabilityError::refused("the frozen skill content is empty")
        })?;
        if frozen_digest != expected_digest {
            return Err(CapabilityError::refused(
                "the skill content changed between the check and the plan",
            ));
        }

        Ok(Self {
            plan_id: format!("plan-{}", uuid::Uuid::now_v7()),
            schema_version: PLAN_SCHEMA_VERSION,
            skill_name,
            source_kind: source.kind().to_string(),
            source_ref: source.redacted_ref(),
            checker_version: report.checker_version.clone(),
            verdict: report.verdict.as_str().to_string(),
            content_digest: expected_digest,
            files: frozen,
            targets: unique,
            frozen_root,
        })
    }

    /// Re-verifies the frozen content before it is copied anywhere.
    pub fn verify_frozen(&self) -> Result<(), CapabilityError> {
        let current = compute_file_manifest(&self.frozen_root)?;
        if current != self.files {
            return Err(CapabilityError::refused(
                "the frozen skill content no longer matches its manifest",
            ));
        }
        Ok(())
    }

    /// Whether a target is part of the frozen selection.
    pub fn includes(&self, target: SkillTargetId) -> bool {
        self.targets.iter().any(|id| id == target.as_str())
    }
}

/// Copies the checked content into the frozen root, hashing every file.
///
/// Only the files the manifest knows about are copied, so an extra file that
/// appeared after the check is never carried into the plan.
fn freeze_content(source_root: &Path, frozen_root: &Path) -> Result<(), CapabilityError> {
    if frozen_root.exists() {
        return Err(CapabilityError::busy(
            "the plan freeze directory already exists",
        ));
    }
    let manifest = compute_file_manifest(source_root)?;
    for entry in &manifest {
        let from = source_root.join(&entry.path);
        let to = frozen_root.join(&entry.path);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                CapabilityError::internal(format!("failed to create the freeze directory: {error}"))
            })?;
        }
        std::fs::copy(&from, &to).map_err(|error| {
            CapabilityError::internal(format!("failed to freeze the skill content: {error}"))
        })?;
        let bytes = std::fs::read(&to).map_err(|error| {
            CapabilityError::internal(format!("failed to read the frozen content: {error}"))
        })?;
        let digest = hex::encode(Sha256::digest(&bytes));
        if digest != entry.sha256 {
            return Err(CapabilityError::refused(format!(
                "the skill content changed while it was being frozen at '{}'",
                entry.path
            )));
        }
    }
    Ok(())
}

/// Mints the operation id an install plan belongs to.
pub fn new_install_operation_id() -> String {
    operation::new_operation_id(crate::capability::types::CapabilityKind::Skill)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::skill::checker::check_directory;
    use tempfile::TempDir;

    fn write_skill(root: &Path) {
        std::fs::create_dir_all(root).expect("create skill dir");
        std::fs::write(root.join("SKILL.md"), "---\nname: demo\n---\n\n# demo\n")
            .expect("write skill");
        std::fs::write(root.join("notes.md"), "prose").expect("write notes");
    }

    fn source() -> SkillSource {
        SkillSource::LocalDirectory {
            path: "/tmp/demo".to_string(),
        }
    }

    #[test]
    fn a_passing_skill_freezes_into_an_applicable_plan() {
        let temp = TempDir::new().expect("temp dir");
        let content = temp.path().join("content");
        write_skill(&content);
        let report = check_directory(&content).expect("check");

        let plan = SkillInstallPlan::build(
            &source(),
            &report,
            &content,
            &[SkillTargetId::Chatspeed],
            temp.path().join("frozen"),
        )
        .expect("plan");

        assert_eq!(plan.skill_name, "demo");
        assert_eq!(plan.verdict, "pass");
        assert_eq!(plan.checker_version, SKILL_CHECKER_VERSION);
        assert_eq!(plan.files.len(), 2);
        assert!(plan.includes(SkillTargetId::Chatspeed));
        assert!(plan.includes(SkillTargetId::Agents) == false);
        plan.verify_frozen().expect("frozen content verifies");
    }

    #[test]
    fn a_duplicate_or_empty_target_selection_is_refused() {
        let temp = TempDir::new().expect("temp dir");
        let content = temp.path().join("content");
        write_skill(&content);
        let report = check_directory(&content).expect("check");

        assert!(SkillInstallPlan::build(
            &source(),
            &report,
            &content,
            &[],
            temp.path().join("frozen-empty"),
        )
        .is_err());
        assert!(SkillInstallPlan::build(
            &source(),
            &report,
            &content,
            &[SkillTargetId::Chatspeed, SkillTargetId::Chatspeed],
            temp.path().join("frozen-dup"),
        )
        .is_err());
    }

    #[test]
    fn content_that_changes_after_the_check_cannot_be_planned() {
        let temp = TempDir::new().expect("temp dir");
        let content = temp.path().join("content");
        write_skill(&content);
        let report = check_directory(&content).expect("check");

        // The source is edited after the check but before the plan is built.
        std::fs::write(content.join("notes.md"), "tampered").expect("tamper");

        let error = SkillInstallPlan::build(
            &source(),
            &report,
            &content,
            &[SkillTargetId::Chatspeed],
            temp.path().join("frozen"),
        )
        .err()
        .expect("a changed source must be refused");
        assert_eq!(error.code(), crate::capability::error::code::REFUSED);
    }

    #[test]
    fn a_frozen_plan_detects_later_tampering_in_the_freeze() {
        let temp = TempDir::new().expect("temp dir");
        let content = temp.path().join("content");
        write_skill(&content);
        let report = check_directory(&content).expect("check");

        let plan = SkillInstallPlan::build(
            &source(),
            &report,
            &content,
            &[SkillTargetId::Chatspeed],
            temp.path().join("frozen"),
        )
        .expect("plan");

        std::fs::write(plan.frozen_root.join("notes.md"), "tampered").expect("tamper");
        let error = plan.verify_frozen().err().expect("must detect tampering");
        assert_eq!(error.code(), crate::capability::error::code::REFUSED);
    }

    #[test]
    fn a_blocked_report_can_never_be_planned() {
        let temp = TempDir::new().expect("temp dir");
        let content = temp.path().join("content");
        std::fs::create_dir_all(&content).expect("create dir");
        std::fs::write(content.join("notes.md"), "no manifest here").expect("write");
        let report = check_directory(&content).expect("check");

        let error = SkillInstallPlan::build(
            &source(),
            &report,
            &content,
            &[SkillTargetId::Chatspeed],
            temp.path().join("frozen"),
        )
        .err()
        .expect("a blocked skill must not be planned");
        assert_eq!(error.code(), crate::capability::error::code::CHECK_BLOCKED);
    }
}
