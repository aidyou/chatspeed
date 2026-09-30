//! Capability doctor: report and (later) converge drift.
//!
//! Doctor is report-only by default. It answers, from durable evidence plus one
//! runtime observation:
//!
//! - is any operation journal row left in flight or requiring reconcile?
//! - did a managed Skill installation drift or lose its directory?
//! - does an MCP server's desired state disagree with what the runtime shows?
//! - is there private staging/quarantine residue?
//!
//! It never deletes non-managed or drifted content, and it never "fixes" an
//! operation whose effect state is unknown by retrying it (INV-6/INV-8).

use std::path::Path;

use serde::Serialize;

use crate::capability::error::CapabilityError;
use crate::capability::mcp_service::McpServerView;
use crate::capability::repository::CapabilityRepository;
use crate::capability::skill_inventory::{SkillInventory, SkillInventorySource};
use crate::capability::{quarantine_dir, staging_dir};

/// Stable finding codes, for adapters that branch on structure.
pub mod finding {
    pub const OPERATION_NEEDS_RECONCILE: &str = "operation_needs_reconcile";
    pub const OPERATION_INTERRUPTED: &str = "operation_interrupted";
    pub const SKILL_DRIFTED: &str = "skill_drifted";
    pub const SKILL_DIRECTORY_MISSING: &str = "skill_directory_missing";
    pub const MCP_DRIFT: &str = "mcp_drift";
    pub const STAGING_RESIDUE: &str = "staging_residue";
    pub const QUARANTINE_RESIDUE: &str = "quarantine_residue";
}

/// Journal-level findings.
#[derive(Debug, Clone, Default, Serialize)]
pub struct JournalFindings {
    /// Operations still in an in-flight state (startup recovery should have
    /// classified these; a non-empty list after startup is itself a finding).
    pub interrupted: Vec<String>,
    /// Operations that require an explicit reconcile decision.
    pub needs_reconcile: Vec<String>,
}

/// Skill-level findings.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SkillFindings {
    pub builtin: usize,
    pub managed: usize,
    pub discovered: usize,
    /// `target:name` of managed installations whose content changed.
    pub drifted: Vec<String>,
    /// `target:name` of managed installations whose directory is gone.
    pub missing: Vec<String>,
    /// Ownership records that exist without a matching on-disk directory.
    pub orphan_ownership: Vec<String>,
}

/// MCP-level findings.
#[derive(Debug, Clone, Default, Serialize)]
pub struct McpFindings {
    pub registered: usize,
    pub desired_enabled: usize,
    /// `name:drift_code` of every server whose desired state disagrees with the
    /// observed runtime.
    pub drift: Vec<String>,
}

/// Private staging/quarantine residue.
#[derive(Debug, Clone, Default, Serialize)]
pub struct StagingFindings {
    pub staging_root: String,
    pub quarantine_root: String,
    pub staging_entries: usize,
    pub quarantine_entries: usize,
}

/// The full doctor report.
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityDoctorReport {
    pub journal: JournalFindings,
    pub skills: SkillFindings,
    pub mcp: McpFindings,
    pub staging: StagingFindings,
    /// The stable finding codes, in report order.
    pub findings: Vec<String>,
}

impl CapabilityDoctorReport {
    /// Whether anything needs attention.
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}

/// Builds the doctor report from durable evidence and one runtime observation.
pub fn build_doctor_report(
    repository: &CapabilityRepository,
    inventory: &SkillInventory,
    mcp_servers: &[McpServerView],
    app_data_dir: &Path,
) -> Result<CapabilityDoctorReport, CapabilityError> {
    let mut findings = Vec::new();

    let journal = JournalFindings {
        interrupted: repository
            .list_interrupted()?
            .into_iter()
            .map(|operation| operation.operation_id)
            .collect(),
        needs_reconcile: repository
            .list_needing_reconcile()?
            .into_iter()
            .map(|operation| operation.operation_id)
            .collect(),
    };
    if !journal.interrupted.is_empty() {
        findings.push(finding::OPERATION_INTERRUPTED.to_string());
    }
    if !journal.needs_reconcile.is_empty() {
        findings.push(finding::OPERATION_NEEDS_RECONCILE.to_string());
    }

    let mut skills = SkillFindings::default();
    for entry in &inventory.skills {
        match entry.source {
            SkillInventorySource::Builtin => skills.builtin += 1,
            SkillInventorySource::Managed => skills.managed += 1,
            SkillInventorySource::ManagedDrifted => {
                let key = format!(
                    "{}:{}",
                    entry.target_id.as_deref().unwrap_or("unknown"),
                    entry.name
                );
                if entry.present {
                    skills.drifted.push(key);
                } else {
                    skills.missing.push(key.clone());
                    skills.orphan_ownership.push(key);
                }
            }
            SkillInventorySource::Discovered => skills.discovered += 1,
        }
    }
    if !skills.drifted.is_empty() {
        findings.push(finding::SKILL_DRIFTED.to_string());
    }
    if !skills.missing.is_empty() {
        findings.push(finding::SKILL_DIRECTORY_MISSING.to_string());
    }

    let mut mcp = McpFindings {
        registered: mcp_servers.len(),
        desired_enabled: mcp_servers
            .iter()
            .filter(|view| view.desired.enabled)
            .count(),
        drift: Vec::new(),
    };
    for view in mcp_servers {
        if let Some(drift) = &view.drift {
            mcp.drift.push(format!("{}:{}", view.name, drift));
        }
    }
    if !mcp.drift.is_empty() {
        findings.push(finding::MCP_DRIFT.to_string());
    }

    let staging_root = staging_dir(app_data_dir);
    let quarantine_root = quarantine_dir(app_data_dir);
    let staging = StagingFindings {
        staging_root: staging_root.to_string_lossy().to_string(),
        quarantine_root: quarantine_root.to_string_lossy().to_string(),
        staging_entries: count_entries(&staging_root),
        quarantine_entries: count_entries(&quarantine_root),
    };
    // Staging is short-lived by contract; residue after a mutation finished is
    // reported so a user (or an explicit cleanup) can remove it. Doctor never
    // deletes it implicitly.
    if staging.staging_entries > 0 {
        findings.push(finding::STAGING_RESIDUE.to_string());
    }
    if staging.quarantine_entries > 0 {
        findings.push(finding::QUARANTINE_RESIDUE.to_string());
    }

    Ok(CapabilityDoctorReport {
        journal,
        skills,
        mcp,
        staging,
        findings,
    })
}

fn count_entries(root: &Path) -> usize {
    match std::fs::read_dir(root) {
        Ok(entries) => entries.flatten().count(),
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::skill_inventory::SkillInventoryService;
    use crate::capability::targets::TargetEnvironment;
    use crate::capability::types::{SkillInstallation, SkillInstallationState};
    use crate::db::MainStore;
    use crate::workflow::react::skills::SkillScanner;
    use std::fs;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[test]
    fn a_clean_capability_state_reports_no_findings() {
        let temp = TempDir::new().expect("temp dir");
        let app_data = temp.path().join("app-data");
        fs::create_dir_all(&app_data).expect("app data");
        let repository = CapabilityRepository::new(Arc::new(
            MainStore::new(":memory:").expect("in-memory store"),
        ));
        let environment = TargetEnvironment::injected(
            temp.path().join("home"),
            temp.path().join("chatspeed"),
        );
        let scanner = SkillScanner::with_search_paths(Vec::new());
        let inventory = SkillInventoryService::new(app_data.clone(), environment)
            .build_with_scanner(&repository, &scanner)
            .expect("inventory");

        let report =
            build_doctor_report(&repository, &inventory, &[], &app_data).expect("doctor report");
        assert!(report.is_clean(), "unexpected findings: {:?}", report.findings);
    }

    #[test]
    fn drift_journal_state_and_residue_are_all_reported() {
        let temp = TempDir::new().expect("temp dir");
        let app_data = temp.path().join("app-data");
        fs::create_dir_all(&app_data).expect("app data");
        let skills_root = temp.path().join("chatspeed").join("skills");
        fs::create_dir_all(&skills_root).expect("skills root");

        let repository = CapabilityRepository::new(Arc::new(
            MainStore::new(":memory:").expect("in-memory store"),
        ));

        // A managed installation whose directory is gone.
        repository
            .upsert_installation(&SkillInstallation {
                installation_id: "skl-gone".to_string(),
                skill_name: "gone".to_string(),
                target_id: "chatspeed".to_string(),
                install_path: skills_root.join("gone").to_string_lossy().to_string(),
                source_kind: "local_directory".to_string(),
                source_ref: "/tmp/source".to_string(),
                checker_version: "skill-checker.v1".to_string(),
                verdict: "pass".to_string(),
                content_digest: "digest".to_string(),
                file_manifest: Vec::new(),
                marker_nonce: "nonce".to_string(),
                manifest_digest: "manifest".to_string(),
                state: SkillInstallationState::Installed,
                operation_id: None,
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .expect("record ownership");

        // Staging residue.
        let staging = crate::capability::staging_dir(&app_data);
        fs::create_dir_all(staging.join("leftover")).expect("staging entry");

        let environment = TargetEnvironment::injected(
            temp.path().join("home"),
            temp.path().join("chatspeed"),
        );
        let scanner = SkillScanner::with_search_paths(vec![skills_root]);
        let inventory = SkillInventoryService::new(app_data.clone(), environment)
            .build_with_scanner(&repository, &scanner)
            .expect("inventory");

        let report =
            build_doctor_report(&repository, &inventory, &[], &app_data).expect("doctor report");
        assert!(report.findings.contains(&finding::SKILL_DIRECTORY_MISSING.to_string()));
        assert!(report.findings.contains(&finding::STAGING_RESIDUE.to_string()));
        assert_eq!(report.skills.missing.len(), 1);
        assert_eq!(report.skills.orphan_ownership.len(), 1);
    }
}
