//! Ownership proof for Agent Skill installations.
//!
//! Two independent proofs must agree before ChatSpeed will ever delete
//! installed content (AC-7/INV-6):
//!
//! 1. the durable `skill_installations` row, written by the install operation;
//! 2. the in-directory marker, which survives a database reset and lets a human
//!    tell a managed directory from one copied in by hand.
//!
//! The marker also carries the content digest, so a directory that was edited
//! after installation is refused as drifted instead of being destroyed.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::capability::error::CapabilityError;
use crate::capability::skill::manifest::MARKER_FILE_NAME;
use crate::capability::types::SkillInstallation;

/// Marker layout version. A future format change must not make today's markers
/// look like foreign files.
pub const MARKER_SCHEMA_VERSION: u32 = 1;

/// The in-directory ownership marker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnershipMarker {
    pub schema_version: u32,
    pub installation_id: String,
    pub skill_name: String,
    pub target_id: String,
    pub checker_version: String,
    pub content_digest: String,
    pub manifest_digest: String,
    /// Random per-install value, so a copied marker cannot claim ownership of a
    /// different directory.
    pub nonce: String,
    pub source_kind: String,
    pub source_ref: String,
    pub installed_at_ms: i64,
}

impl OwnershipMarker {
    /// Builds the marker that matches one installation row.
    pub fn from_installation(installation: &SkillInstallation) -> Self {
        Self {
            schema_version: MARKER_SCHEMA_VERSION,
            installation_id: installation.installation_id.clone(),
            skill_name: installation.skill_name.clone(),
            target_id: installation.target_id.clone(),
            checker_version: installation.checker_version.clone(),
            content_digest: installation.content_digest.clone(),
            manifest_digest: installation.manifest_digest.clone(),
            nonce: installation.marker_nonce.clone(),
            source_kind: installation.source_kind.clone(),
            source_ref: installation.source_ref.clone(),
            installed_at_ms: installation.created_at_ms,
        }
    }

    /// Whether this marker is the marker of that installation row.
    ///
    /// Every field that identifies the content, the target and the specific
    /// install must match; a stale or transplanted marker is not ownership.
    pub fn matches(&self, installation: &SkillInstallation) -> bool {
        self.schema_version == MARKER_SCHEMA_VERSION
            && self.installation_id == installation.installation_id
            && self.skill_name == installation.skill_name
            && self.target_id == installation.target_id
            && self.content_digest == installation.content_digest
            && self.manifest_digest == installation.manifest_digest
            && self.nonce == installation.marker_nonce
    }
}

/// The marker path inside one installed skill directory.
pub fn marker_path(root: &Path) -> std::path::PathBuf {
    root.join(MARKER_FILE_NAME)
}

/// Writes the marker into an installed skill directory.
pub fn write_marker(root: &Path, marker: &OwnershipMarker) -> Result<(), CapabilityError> {
    let path = marker_path(root);
    let payload = serde_json::to_vec_pretty(marker)?;
    std::fs::write(&path, payload).map_err(|error| {
        CapabilityError::internal(format!("failed to write the ownership marker: {error}"))
    })
}

/// Reads the marker of one installed skill directory.
///
/// A missing marker is `None` (the directory is simply not managed by us); a
/// present but unreadable or invalid marker is an error, because it must not be
/// silently treated as "not ours" when it may be the proof of ownership.
pub fn read_marker(root: &Path) -> Result<Option<OwnershipMarker>, CapabilityError> {
    let path = marker_path(root);
    if !path.exists() {
        return Ok(None);
    }
    if !path.is_file() {
        return Err(CapabilityError::refused(
            "the ownership marker is not a regular file",
        ));
    }
    let bytes = std::fs::read(&path).map_err(|error| {
        CapabilityError::internal(format!("failed to read the ownership marker: {error}"))
    })?;
    let marker: OwnershipMarker = serde_json::from_slice(&bytes).map_err(|_| {
        CapabilityError::refused("the ownership marker is not a valid ChatSpeed marker")
    })?;
    Ok(Some(marker))
}

/// Whether the directory carries the marker of exactly this installation.
pub fn has_proof(root: &Path, installation: &SkillInstallation) -> Result<bool, CapabilityError> {
    match read_marker(root)? {
        Some(marker) => Ok(marker.matches(installation)),
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::types::SkillInstallationState;
    use tempfile::TempDir;

    fn installation(nonce: &str, digest: &str) -> SkillInstallation {
        SkillInstallation {
            installation_id: "ins-1".to_string(),
            skill_name: "demo".to_string(),
            target_id: "chatspeed".to_string(),
            install_path: "/tmp/demo".to_string(),
            source_kind: "local_directory".to_string(),
            source_ref: "local_directory:demo".to_string(),
            checker_version: "skill-checker.v1".to_string(),
            verdict: "pass".to_string(),
            content_digest: digest.to_string(),
            file_manifest: Vec::new(),
            marker_nonce: nonce.to_string(),
            manifest_digest: digest.to_string(),
            state: SkillInstallationState::Installed,
            operation_id: Some("op-skill-1".to_string()),
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    #[test]
    fn a_written_marker_proves_ownership_and_a_transplanted_one_does_not() {
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("demo");
        std::fs::create_dir_all(&root).expect("create dir");

        let row = installation("nonce-a", "digest-a");
        assert!(!has_proof(&root, &row).expect("no marker yet"));

        write_marker(&root, &OwnershipMarker::from_installation(&row)).expect("write marker");
        assert!(has_proof(&root, &row).expect("marker proves ownership"));

        // A different nonce or digest is not the same install.
        assert!(!has_proof(&root, &installation("nonce-b", "digest-a")).expect("nonce mismatch"));
        assert!(!has_proof(&root, &installation("nonce-a", "digest-b")).expect("digest mismatch"));
    }

    #[test]
    fn a_corrupt_marker_is_refused_rather_than_treated_as_absent() {
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("demo");
        std::fs::create_dir_all(&root).expect("create dir");
        std::fs::write(marker_path(&root), b"{not json").expect("write");

        let error = read_marker(&root).expect_err("corrupt marker must not be ignored");
        assert_eq!(error.code(), crate::capability::error::code::REFUSED);
    }
}
