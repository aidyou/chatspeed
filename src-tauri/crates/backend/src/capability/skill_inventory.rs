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
use crate::capability::skill::uninstaller::is_uninstall_eligible_target;
use crate::capability::target_preflight;
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
    /// Whether the entry is protected from overwrite and removal: a bundled
    /// Skill, a reserved name, or a directory that is not under a registered
    /// target.
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
    pub fn build(
        &self,
        repository: &CapabilityRepository,
    ) -> Result<SkillInventory, CapabilityError> {
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
        let scanned = scanner
            .scan_detailed()
            .map_err(|error| CapabilityError::internal(format!("skill scan failed: {error}")))?;

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

            // Only a directory under a registered target can ever be removed, so
            // one that maps to no registered target is surfaced as protected and
            // can never be uninstallable.
            let registered_target = target_id
                .as_deref()
                .map(is_uninstall_eligible_target)
                .unwrap_or(false);
            let protected = skill.builtin
                || RESERVED_SKILL_NAMES.contains(&skill.manifest.name.as_str())
                || !registered_target;

            let (source, drifted) =
                classify(skill.builtin, installation.as_ref(), &skill.directory);

            let installation_state = installation
                .as_ref()
                .map(|found| found.state.as_str().to_string());
            // A managed directory is uninstallable while it is a proven install;
            // a discovered directory (no ownership row) is uninstallable because
            // the caller explicitly selected this exact `(target, name)`. Both
            // still require a registered target and no drift.
            let uninstallable = !protected
                && !drifted
                && match &installation {
                    None => true,
                    Some(found) => found.state == SkillInstallationState::Installed,
                };

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

        // An ownership row the scan did not claim is still part of the truth:
        // doctor must see it, and an external target the scanner does not read
        // must still surface its install. Drift is re-verified here so an
        // unread target never offers a drifted directory for uninstall.
        for installation in &installations {
            if claimed.contains(&installation.installation_id) {
                continue;
            }
            let install_path = Path::new(&installation.install_path);
            let present = install_path.is_dir();
            if installation.state == SkillInstallationState::Removed && !present {
                continue;
            }
            let drifted = !present
                || !matches!(
                    verify_file_manifest(install_path, &installation.file_manifest),
                    Ok(ManifestVerdict::Match)
                );
            let protected = RESERVED_SKILL_NAMES.contains(&installation.skill_name.as_str())
                || !is_uninstall_eligible_target(&installation.target_id);
            skills.push(SkillInventoryEntry {
                name: installation.skill_name.clone(),
                version: String::new(),
                description: String::new(),
                source: if drifted {
                    SkillInventorySource::ManagedDrifted
                } else {
                    SkillInventorySource::Managed
                },
                target_id: Some(installation.target_id.clone()),
                directory: present.then(|| installation.install_path.clone()),
                present,
                managed: true,
                drifted,
                protected,
                uninstallable: present
                    && !protected
                    && !drifted
                    && installation.state == SkillInstallationState::Installed,
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

/// Maps a scanned search root back to the registered target that canonically
/// owns its physical directory.
///
/// Several registry ids can resolve to one physical directory — for example
/// `chatspeed` when `CHATSPEED_HOME` points at the shared Agents home — so a
/// lexical match alone would attribute a canonical Agents directory to the
/// `chatspeed` alias. Resolution uses the same canonical-owner rule as the write
/// path, otherwise one installation is reported twice: once as discovered under
/// the alias and once as managed under its real owner (AC-7/INV-2).
fn target_for_root(targets: &[ResolvedSkillTarget], root: &Path) -> Option<String> {
    target_preflight::canonical_owner_for_root(targets, root)
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
        let chatspeed_home = temp.path().join("chatspeed");
        fixture_in(temp, chatspeed_home)
    }

    /// A fixture whose `CHATSPEED_HOME` points at the shared Agents home, so the
    /// `chatspeed` and `agents` ids resolve to one physical directory.
    fn aliased_fixture() -> Fixture {
        let temp = TempDir::new().expect("temp dir");
        let chatspeed_home = temp.path().join("home").join(".agents");
        fixture_in(temp, chatspeed_home)
    }

    fn fixture_in(temp: TempDir, chatspeed_home: PathBuf) -> Fixture {
        let home = temp.path().join("home");
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
        assert!(
            !loose.protected,
            "a discovered Skill under a registered target"
        );
        assert!(
            loose.uninstallable,
            "a discovered Skill is uninstallable when the user selects its exact directory"
        );
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
        let manifest =
            crate::capability::skill::manifest::compute_file_manifest(&reserved).expect("manifest");
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
    fn an_external_target_install_is_managed_and_uninstallable() {
        let fixture = fixture();
        let codex_root = fixture
            .environment
            .home_dir
            .clone()
            .expect("home")
            .join(".codex")
            .join("skills");
        fs::create_dir_all(&codex_root).expect("create codex skills dir");
        let directory = write_skill(&codex_root, "external-skill", "body");
        let manifest = crate::capability::skill::manifest::compute_file_manifest(&directory)
            .expect("manifest");
        let mut installation = installation_for("external-skill", &directory, manifest);
        installation.target_id = "codex".to_string();
        fixture
            .repository
            .upsert_installation(&installation)
            .expect("record ownership");

        let scanner =
            SkillScanner::with_search_paths(vec![fixture.chatspeed_skills.clone(), codex_root]);
        let service =
            SkillInventoryService::new(PathBuf::from("/tmp/app-data"), fixture.environment.clone());
        let inventory = service
            .build_with_scanner(&fixture.repository, &scanner)
            .expect("inventory");

        let entry = inventory
            .skills
            .iter()
            .find(|entry| entry.name == "external-skill")
            .expect("external entry");
        assert_eq!(entry.target_id.as_deref(), Some("codex"));
        assert_eq!(entry.source, SkillInventorySource::Managed);
        assert!(entry.managed);
        assert!(!entry.drifted);
        assert!(!entry.protected, "an external target install is deletable");
        assert!(entry.uninstallable);
        assert!(
            inventory
                .uninstallable()
                .iter()
                .any(|candidate| candidate.name == "external-skill"),
            "an external entry is offered for uninstall"
        );
    }

    #[test]
    fn an_external_discovered_directory_is_uninstallable() {
        let fixture = fixture();
        let codex_root = fixture
            .environment
            .home_dir
            .clone()
            .expect("home")
            .join(".codex")
            .join("skills");
        fs::create_dir_all(&codex_root).expect("create codex skills dir");
        write_skill(&codex_root, "external-loose", "body");

        let scanner =
            SkillScanner::with_search_paths(vec![fixture.chatspeed_skills.clone(), codex_root]);
        let service =
            SkillInventoryService::new(PathBuf::from("/tmp/app-data"), fixture.environment.clone());
        let inventory = service
            .build_with_scanner(&fixture.repository, &scanner)
            .expect("inventory");

        let entry = inventory
            .skills
            .iter()
            .find(|entry| entry.name == "external-loose")
            .expect("external discovered entry");
        assert_eq!(entry.target_id.as_deref(), Some("codex"));
        assert_eq!(entry.source, SkillInventorySource::Discovered);
        assert!(!entry.managed);
        assert!(!entry.protected);
        assert!(entry.uninstallable);
    }

    #[test]
    fn an_external_managed_row_the_scanner_does_not_read_is_still_uninstallable() {
        let fixture = fixture();
        // The scanner only reads the ChatSpeed root, so the external install is
        // reported from its ownership row rather than a scan.
        let codex_root = fixture
            .environment
            .home_dir
            .clone()
            .expect("home")
            .join(".codex")
            .join("skills");
        fs::create_dir_all(&codex_root).expect("create codex skills dir");
        let directory = write_skill(&codex_root, "unscanned-external", "body");
        let manifest = crate::capability::skill::manifest::compute_file_manifest(&directory)
            .expect("manifest");
        let mut installation = installation_for("unscanned-external", &directory, manifest);
        installation.target_id = "codex".to_string();
        fixture
            .repository
            .upsert_installation(&installation)
            .expect("record ownership");

        let service =
            SkillInventoryService::new(PathBuf::from("/tmp/app-data"), fixture.environment.clone());
        let inventory = service
            .build_with_scanner(&fixture.repository, &fixture.scanner)
            .expect("inventory");

        let entry = inventory
            .skills
            .iter()
            .find(|entry| entry.name == "unscanned-external")
            .expect("unscanned external entry");
        assert!(entry.present);
        assert_eq!(entry.source, SkillInventorySource::Managed);
        assert!(!entry.drifted);
        assert!(!entry.protected);
        assert!(entry.uninstallable);
    }

    #[test]
    fn an_unscanned_external_row_with_drift_is_not_uninstallable() {
        let fixture = fixture();
        let codex_root = fixture
            .environment
            .home_dir
            .clone()
            .expect("home")
            .join(".codex")
            .join("skills");
        fs::create_dir_all(&codex_root).expect("create codex skills dir");
        let directory = write_skill(&codex_root, "unscanned-drift", "body");
        let manifest = crate::capability::skill::manifest::compute_file_manifest(&directory)
            .expect("manifest");
        let mut installation = installation_for("unscanned-drift", &directory, manifest);
        installation.target_id = "codex".to_string();
        fixture
            .repository
            .upsert_installation(&installation)
            .expect("record ownership");
        fs::write(
            directory.join("SKILL.md"),
            "---\nname: unscanned-drift\ndescription: edited\n---\nedited",
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
            .find(|entry| entry.name == "unscanned-drift")
            .expect("unscanned drifted entry");
        assert_eq!(entry.source, SkillInventorySource::ManagedDrifted);
        assert!(entry.drifted);
        assert!(!entry.uninstallable);
    }

    #[test]
    fn a_skill_outside_every_registered_target_is_protected_and_not_uninstallable() {
        let fixture = fixture();
        // A bundled-resource root (and any root that is not a registered target)
        // resolves to no target, so its Skill can never be offered for removal.
        let bundled_root = fixture._temp.path().join("bundled").join("skills");
        fs::create_dir_all(&bundled_root).expect("create bundled root");
        write_skill(&bundled_root, "builtin-help", "body");

        let scanner = SkillScanner::with_search_paths(vec![
            fixture.chatspeed_skills.clone(),
            bundled_root.clone(),
        ]);
        let service =
            SkillInventoryService::new(PathBuf::from("/tmp/app-data"), fixture.environment.clone());
        let inventory = service
            .build_with_scanner(&fixture.repository, &scanner)
            .expect("inventory");

        let entry = inventory
            .skills
            .iter()
            .find(|entry| entry.name == "builtin-help")
            .expect("bundled entry");
        assert_eq!(entry.target_id, None);
        assert!(entry.protected);
        assert!(!entry.uninstallable);
        assert!(bundled_root.join("builtin-help/SKILL.md").is_file());
    }

    #[test]
    fn a_canonical_agents_install_remains_uninstallable() {
        let fixture = fixture();
        let agents_root = fixture
            .environment
            .home_dir
            .clone()
            .expect("home")
            .join(".agents")
            .join("skills");
        fs::create_dir_all(&agents_root).expect("create agents skills dir");
        let directory = write_skill(&agents_root, "agents-skill", "body");
        let manifest = crate::capability::skill::manifest::compute_file_manifest(&directory)
            .expect("manifest");
        let mut installation = installation_for("agents-skill", &directory, manifest);
        installation.target_id = "agents".to_string();
        fixture
            .repository
            .upsert_installation(&installation)
            .expect("record ownership");

        let scanner =
            SkillScanner::with_search_paths(vec![fixture.chatspeed_skills.clone(), agents_root]);
        let service =
            SkillInventoryService::new(PathBuf::from("/tmp/app-data"), fixture.environment.clone());
        let inventory = service
            .build_with_scanner(&fixture.repository, &scanner)
            .expect("inventory");

        let entry = inventory
            .skills
            .iter()
            .find(|entry| entry.name == "agents-skill")
            .expect("agents entry");
        assert_eq!(entry.target_id.as_deref(), Some("agents"));
        assert!(entry.managed);
        assert!(!entry.protected, "the canonical Agents target is deletable");
        assert!(entry.uninstallable);
    }

    #[test]
    fn an_aliased_home_reports_one_managed_identity_for_the_canonical_owner() {
        let fixture = aliased_fixture();
        // The only row is the canonical Agents owner of this physical directory,
        // even though the alias home makes `chatspeed` resolve to it as well.
        let directory = write_skill(&fixture.chatspeed_skills, "demo", "body");
        let manifest = crate::capability::skill::manifest::compute_file_manifest(&directory)
            .expect("manifest");
        let mut installation = installation_for("demo", &directory, manifest);
        installation.target_id = "agents".to_string();
        fixture
            .repository
            .upsert_installation(&installation)
            .expect("record ownership");

        let service =
            SkillInventoryService::new(PathBuf::from("/tmp/app-data"), fixture.environment.clone());
        let inventory = service
            .build_with_scanner(&fixture.repository, &fixture.scanner)
            .expect("inventory");

        let entries: Vec<_> = inventory
            .skills
            .iter()
            .filter(|entry| entry.name == "demo")
            .collect();
        assert_eq!(
            entries.len(),
            1,
            "one physical installation is reported once, not once per aliasing id"
        );
        let entry = entries[0];
        assert_eq!(
            entry.target_id.as_deref(),
            Some("agents"),
            "the canonical owner is reported, not the chatspeed alias"
        );
        assert_eq!(entry.source, SkillInventorySource::Managed);
        assert!(entry.managed);
        assert!(!entry.drifted);
        assert!(
            entry.uninstallable,
            "the canonical owner remains uninstallable"
        );
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
