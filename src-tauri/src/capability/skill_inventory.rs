//! Read-only Agent Skill inventory.
//!
//! The inventory answers one question for every adapter: *what Skill exists on
//! disk, who owns it, and may ChatSpeed touch it?* It reuses the workflow
//! `SkillScanner` for parsing (so the resolver precedence has exactly one
//! implementation) and adds the classification the scanner intentionally does
//! not have: bundled, managed-by-ChatSpeed, drifted, or merely discovered.
//!
//! Nothing here mutates a directory or a journal row.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::capability::error::CapabilityError;
use crate::capability::repository::CapabilityRepository;
use crate::capability::skill::manifest::{verify_file_manifest, ManifestVerdict};
use crate::capability::targets::{
    resolve_targets, ResolvedSkillTarget, SkillTargetId, TargetEnvironment,
};
use crate::capability::types::{SkillInstallation, SkillInstallationState};
use crate::workflow::react::skills::SkillScanner;

/// Skill names that are reserved and must never be installed over or removed.
///
/// `chatspeed-cli` is the CLI capability name; a directory with that name is
/// protected even when it is not bundled (INV-6/AC-7).
pub const RESERVED_SKILL_NAMES: &[&str] = &["chatspeed-cli"];

/// Where an inventory entry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillInventorySource {
    /// Shipped with the application; never installable over and never removed.
    Builtin,
    /// A ChatSpeed-managed installation whose content still matches its proof.
    Managed,
    /// A ChatSpeed-managed installation whose content changed, or whose
    /// directory disappeared.
    ManagedDrifted,
    /// Present on disk without a ChatSpeed ownership record.
    Discovered,
}

impl SkillInventorySource {
    pub fn as_str(&self) -> &'static str {
        match self {
            SkillInventorySource::Builtin => "builtin",
            SkillInventorySource::Managed => "managed",
            SkillInventorySource::ManagedDrifted => "managed_drifted",
            SkillInventorySource::Discovered => "discovered",
        }
    }
}

/// One Skill as reported to an adapter.
#[derive(Debug, Clone, Serialize)]
pub struct SkillInventoryEntry {
    pub name: String,
    pub version: String,
    pub description: String,
    pub source: SkillInventorySource,
    /// The install target the directory belongs to, when it maps to one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    /// The skill directory, when it exists on disk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
    /// Whether the recorded installation directory still exists.
    pub present: bool,
    /// Whether ChatSpeed holds an ownership record for this entry.
    pub managed: bool,
    /// Whether the content no longer matches the ownership proof.
    pub drifted: bool,
    /// Whether the entry is protected from overwrite and removal.
    pub protected: bool,
    /// Whether an uninstall of this entry is currently permitted.
    pub uninstallable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installation_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checker_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<String>,
}

/// The whole read model: every target and every Skill.
#[derive(Debug, Clone, Serialize)]
pub struct SkillInventory {
    pub targets: Vec<ResolvedSkillTarget>,
    pub skills: Vec<SkillInventoryEntry>,
}

impl SkillInventory {
    /// Skills that a user could actually uninstall right now.
    pub fn uninstallable(&self) -> Vec<&SkillInventoryEntry> {
        self.skills
            .iter()
            .filter(|entry| entry.uninstallable)
            .collect()
    }
}

/// Builds inventories for the ChatSpeed app data directory.
pub struct SkillInventoryService {
    app_data_dir: PathBuf,
    environment: TargetEnvironment,
}

impl SkillInventoryService {
    pub fn new(app_data_dir: PathBuf, environment: TargetEnvironment) -> Self {
        Self {
            app_data_dir,
            environment,
        }
    }

    /// The target environment this service resolves against.
    pub fn environment(&self) -> &TargetEnvironment {
        &self.environment
    }

    /// Builds the inventory using the default scanner for this data directory.
    pub fn build(&self, repository: &CapabilityRepository) -> Result<SkillInventory, CapabilityError> {
        let scanner = SkillScanner::new(self.app_data_dir.clone());
        self.build_with_scanner(repository, &scanner)
    }

    /// Builds the inventory over an explicit scanner.
    ///
    /// Split out so an isolated test (or a hosted run) can resolve skills from
    /// an injected search-path list instead of the process HOME.
    pub fn build_with_scanner(
        &self,
        repository: &CapabilityRepository,
        scanner: &SkillScanner,
    ) -> Result<SkillInventory, CapabilityError> {
        let targets = resolve_targets(&self.environment);
        let installations = repository.list_installations()?;
        let scanned = scanner.scan_detailed().map_err(|error| {
            CapabilityError::internal(format!("skill scan failed: {error}"))
        })?;

        let mut skills = Vec::new();
        let mut claimed: HashSet<String> = HashSet::new();

        for skill in scanned {
            let target_id = target_for_root(&targets, &skill.root);
            let installation = target_id.as_deref().and_then(|target| {
                installations
                    .iter()
                    .find(|candidate| {
                        candidate.target_id == target && candidate.skill_name == skill.manifest.name
                    })
                    .cloned()
            });
            if let Some(found) = &installation {
                claimed.insert(found.installation_id.clone());
            }

            let protected = skill.builtin
                || RESERVED_SKILL_NAMES.contains(&skill.manifest.name.as_str());

            let (source, drifted) = classify(
                skill.builtin,
                installation.as_ref(),
                &skill.directory,
            );

            let installation_state = installation
                .as_ref()
                .map(|found| found.state.as_str().to_string());
            let uninstallable = !protected
                && !drifted
                && installation
                    .as_ref()
                    .map(|found| found.state == SkillInstallationState::Installed)
                    .unwrap_or(false);

            skills.push(SkillInventoryEntry {
                name: skill.manifest.name.clone(),
                version: skill.manifest.version.clone(),
                description: skill.manifest.description.clone(),
                source,
                target_id,
                directory: Some(skill.directory.to_string_lossy().to_string()),
                present: true,
                managed: installation.is_some(),
                drifted,
                protected,
                uninstallable,
                installation_state,
                checker_version: installation
                    .as_ref()
                    .map(|found| found.checker_version.clone()),
                verdict: installation.as_ref().map(|found| found.verdict.clone()),
            });
        }

        // An ownership record whose directory is gone is still part of the
        // truth: doctor must see it instead of silently losing the evidence.
        for installation in &installations {
            if claimed.contains(&installation.installation_id) {
                continue;
            }
            let present = Path::new(&installation.install_path).is_dir();
            if installation.state == SkillInstallationState::Removed && !present {
                continue;
            }
            skills.push(SkillInventoryEntry {
                name: installation.skill_name.clone(),
                version: String::new(),
                description: String::new(),
                source: if present {
                    SkillInventorySource::Managed
                } else {
                    SkillInventorySource::ManagedDrifted
                },
                target_id: Some(installation.target_id.clone()),
                directory: present.then(|| installation.install_path.clone()),
                present,
                managed: true,
                drifted: !present,
                protected: RESERVED_SKILL_NAMES.contains(&installation.skill_name.as_str()),
                uninstallable: false,
                installation_state: Some(installation.state.as_str().to_string()),
                checker_version: Some(installation.checker_version.clone()),
                verdict: Some(installation.verdict.clone()),
            });
        }

        skills.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.target_id.cmp(&right.target_id))
        });

        Ok(SkillInventory { targets, skills })
    }
}

/// Classifies one scanned Skill against its ownership record.
fn classify(
    builtin: bool,
    installation: Option<&SkillInstallation>,
    directory: &Path,
) -> (SkillInventorySource, bool) {
    if builtin {
        return (SkillInventorySource::Builtin, false);
    }
    let Some(installation) = installation else {
        return (SkillInventorySource::Discovered, false);
    };

    match installation.state {
        SkillInstallationState::Installed => {
            match verify_file_manifest(directory, &installation.file_manifest) {
                Ok(ManifestVerdict::Match) => (SkillInventorySource::Managed, false),
                Ok(ManifestVerdict::Drifted { .. }) | Err(_) => {
                    (SkillInventorySource::ManagedDrifted, true)
                }
            }
        }
        SkillInstallationState::Installing => (SkillInventorySource::Managed, false),
        SkillInstallationState::Drifted => (SkillInventorySource::ManagedDrifted, true),
        SkillInstallationState::Quarantined | SkillInstallationState::Removed => {
            (SkillInventorySource::ManagedDrifted, true)
        }
    }
}

/// Maps a scanned search root back to a registered target id.
fn target_for_root(targets: &[ResolvedSkillTarget], root: &Path) -> Option<String> {
    let root = root.to_string_lossy();
    targets
        .iter()
        .find(|target| {
            target
                .path
                .as_deref()
                .map(|path| paths_equal(path, &root))
                .unwrap_or(false)
        })
        .map(|target| target.id.clone())
}

fn paths_equal(left: &str, right: &str) -> bool {
    let left = left.trim_end_matches(std::path::MAIN_SEPARATOR);
    let right = right.trim_end_matches(std::path::MAIN_SEPARATOR);
    left == right
}

/// The target id a managed installation belongs to, for adapter display.
pub fn target_id_of(installation: &SkillInstallation) -> SkillTargetId {
    SkillTargetId::parse(&installation.target_id).unwrap_or(SkillTargetId::Chatspeed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::types::SkillFileEntry;
    use std::fs;
    use tempfile::TempDir;

    struct Fixture {
        _temp: TempDir,
        environment: TargetEnvironment,
        scanner: SkillScanner,
        repository: CapabilityRepository,
        chatspeed_skills: PathBuf,
    }

    fn fixture() -> Fixture {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path().join("home");
        let chatspeed_home = temp.path().join("chatspeed");
        let app_data = temp.path().join("app-data");
        let chatspeed_skills = chatspeed_home.join("skills");
        fs::create_dir_all(&chatspeed_skills).expect("create chatspeed skills dir");
        fs::create_dir_all(&app_data).expect("create app data dir");

        let environment = TargetEnvironment::injected(home, chatspeed_home);
        let scanner = SkillScanner::with_search_paths(vec![chatspeed_skills.clone()]);
        let repository = CapabilityRepository::new(std::sync::Arc::new(
            crate::db::MainStore::new(":memory:").expect("in-memory store"),
        ));

        Fixture {
            _temp: temp,
            environment,
            scanner,
            repository,
            chatspeed_skills,
        }
    }

    fn write_skill(root: &Path, name: &str, body: &str) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(&dir).expect("create skill dir");
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: test skill\n---\n{body}"),
        )
        .expect("write skill");
        dir
    }

    fn installation_for(
        name: &str,
        directory: &Path,
        manifest: Vec<SkillFileEntry>,
    ) -> SkillInstallation {
        SkillInstallation {
            installation_id: format!("skl-{name}"),
            skill_name: name.to_string(),
            target_id: "chatspeed".to_string(),
            install_path: directory.to_string_lossy().to_string(),
            source_kind: "local_directory".to_string(),
            source_ref: "/tmp/source".to_string(),
            checker_version: "skill-checker.v1".to_string(),
            verdict: "pass".to_string(),
            content_digest: "digest".to_string(),
            file_manifest: manifest,
            marker_nonce: "nonce".to_string(),
            manifest_digest: "manifest".to_string(),
            state: SkillInstallationState::Installed,
            operation_id: None,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    #[test]
    fn an_untracked_directory_is_discovered_and_a_managed_one_is_managed() {
        let fixture = fixture();
        let managed_dir = write_skill(&fixture.chatspeed_skills, "managed-skill", "body");
        write_skill(&fixture.chatspeed_skills, "loose-skill", "body");

        let manifest = crate::capability::skill::manifest::compute_file_manifest(&managed_dir)
            .expect("manifest");
        fixture
            .repository
            .upsert_installation(&installation_for("managed-skill", &managed_dir, manifest))
            .expect("record ownership");

        let service =
            SkillInventoryService::new(PathBuf::from("/tmp/app-data"), fixture.environment.clone());
        let inventory = service
            .build_with_scanner(&fixture.repository, &fixture.scanner)
            .expect("inventory");

        let managed = inventory
            .skills
            .iter()
            .find(|entry| entry.name == "managed-skill")
            .expect("managed entry");
        assert_eq!(managed.source, SkillInventorySource::Managed);
        assert!(managed.managed);
        assert!(!managed.drifted);
        assert!(managed.uninstallable);
        assert_eq!(managed.target_id.as_deref(), Some("chatspeed"));

        let loose = inventory
            .skills
            .iter()
            .find(|entry| entry.name == "loose-skill")
            .expect("discovered entry");
        assert_eq!(loose.source, SkillInventorySource::Discovered);
        assert!(!loose.managed);
        assert!(!loose.uninstallable);
    }

    #[test]
    fn edited_managed_content_is_reported_as_drifted_and_not_uninstallable() {
        let fixture = fixture();
        let directory = write_skill(&fixture.chatspeed_skills, "drift-skill", "body");
        let manifest = crate::capability::skill::manifest::compute_file_manifest(&directory)
            .expect("manifest");
        fixture
            .repository
            .upsert_installation(&installation_for("drift-skill", &directory, manifest))
            .expect("record ownership");

        fs::write(
            directory.join("SKILL.md"),
            "---\nname: drift-skill\ndescription: edited\n---\nedited",
        )
        .expect("edit skill");

        let service =
            SkillInventoryService::new(PathBuf::from("/tmp/app-data"), fixture.environment.clone());
        let inventory = service
            .build_with_scanner(&fixture.repository, &fixture.scanner)
            .expect("inventory");
        let entry = inventory
            .skills
            .iter()
            .find(|entry| entry.name == "drift-skill")
            .expect("drifted entry");
        assert_eq!(entry.source, SkillInventorySource::ManagedDrifted);
        assert!(entry.drifted);
        assert!(!entry.uninstallable);
    }

    #[test]
    fn a_reserved_name_is_protected_and_a_missing_directory_is_still_reported() {
        let fixture = fixture();
        let reserved = write_skill(&fixture.chatspeed_skills, "chatspeed-cli", "body");
        let manifest = crate::capability::skill::manifest::compute_file_manifest(&reserved)
            .expect("manifest");
        fixture
            .repository
            .upsert_installation(&installation_for("chatspeed-cli", &reserved, manifest))
            .expect("record ownership");

        // An ownership record whose directory vanished stays visible.
        let gone = fixture.chatspeed_skills.join("gone-skill");
        fixture
            .repository
            .upsert_installation(&installation_for("gone-skill", &gone, Vec::new()))
            .expect("record ownership");

        let service =
            SkillInventoryService::new(PathBuf::from("/tmp/app-data"), fixture.environment.clone());
        let inventory = service
            .build_with_scanner(&fixture.repository, &fixture.scanner)
            .expect("inventory");

        let reserved_entry = inventory
            .skills
            .iter()
            .find(|entry| entry.name == "chatspeed-cli")
            .expect("reserved entry");
        assert!(reserved_entry.protected);
        assert!(!reserved_entry.uninstallable);

        let gone_entry = inventory
            .skills
            .iter()
            .find(|entry| entry.name == "gone-skill")
            .expect("missing entry");
        assert!(!gone_entry.present);
        assert!(gone_entry.drifted);
    }

    #[test]
    fn the_inventory_lists_every_registered_target() {
        let fixture = fixture();
        let service =
            SkillInventoryService::new(PathBuf::from("/tmp/app-data"), fixture.environment.clone());
        let inventory = service
            .build_with_scanner(&fixture.repository, &fixture.scanner)
            .expect("inventory");
        assert_eq!(inventory.targets.len(), SkillTargetId::ALL.len());
        assert_eq!(
            inventory
                .targets
                .iter()
                .filter(|target| target.default_selected)
                .count(),
            1
        );
    }
}
