//! On-disk content manifests for Agent Skill directories.
//!
//! A manifest is the ownership proof behind drift detection and uninstall: it
//! records every regular file under a skill directory with its SHA-256, so a
//! later verification can tell "unchanged, managed content" from "someone
//! edited it". Special files are refused rather than skipped, because a
//! symlink or device node inside a skill directory is not something ChatSpeed
//! will ever claim to manage (INV-6).

use std::collections::BTreeMap;
use std::path::Path;

use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::capability::error::CapabilityError;
use crate::capability::types::SkillFileEntry;

/// Upper bound on the files one manifest will record.
pub const MAX_MANIFEST_FILES: usize = 4_096;
/// Upper bound on the total bytes one manifest will read.
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;

/// The ownership marker is never part of the content manifest: it is written
/// *after* the content it describes, and counting it would make every managed
/// directory look drifted.
pub const MARKER_FILE_NAME: &str = ".chatspeed-skill.json";

/// Whether a root-relative path is the ownership marker.
pub fn is_marker_path(relative: &str) -> bool {
    relative == MARKER_FILE_NAME
}

/// A stable digest of a file manifest, used as the content identity.
///
/// Delegates to the shared canonical digest so a change to one implementation
/// can never make the checker and the drift check disagree.
pub fn manifest_digest(entries: &[SkillFileEntry]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let pairs: Vec<(String, String)> = entries
        .iter()
        .map(|entry| (entry.path.clone(), entry.sha256.clone()))
        .collect();
    Some(crate::capability::operation::content_digest(&pairs))
}

/// The result of comparing a directory against a recorded manifest.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum ManifestVerdict {
    /// The directory matches the recorded manifest exactly.
    Match,
    /// The directory changed.
    Drifted {
        missing: Vec<String>,
        added: Vec<String>,
        changed: Vec<String>,
    },
}

impl ManifestVerdict {
    pub fn is_match(&self) -> bool {
        matches!(self, ManifestVerdict::Match)
    }
}

/// Computes the manifest of a skill directory.
///
/// Directories are walked without following links; a symlink, device node or
/// other non-regular file is a hard error so the caller fails closed instead of
/// recording a partial proof.
pub fn compute_file_manifest(root: &Path) -> Result<Vec<SkillFileEntry>, CapabilityError> {
    if !root.is_dir() {
        return Err(CapabilityError::internal(format!(
            "skill directory is not a readable directory: {}",
            root.display()
        )));
    }

    let mut entries = Vec::new();
    let mut total_bytes: u64 = 0;

    for entry in WalkDir::new(root).follow_links(false).sort_by_file_name() {
        let entry = entry.map_err(|error| {
            CapabilityError::internal(format!("failed to walk skill directory: {error}"))
        })?;
        if entry.depth() == 0 {
            continue;
        }

        let file_type = entry.file_type();
        if file_type.is_symlink() {
            return Err(CapabilityError::refused(format!(
                "skill content contains a symbolic link at '{}'",
                relative_path(root, entry.path()).unwrap_or_else(|| "<unknown>".to_string())
            )));
        }
        if file_type.is_dir() {
            continue;
        }
        if !file_type.is_file() {
            return Err(CapabilityError::refused(format!(
                "skill content contains a special file at '{}'",
                relative_path(root, entry.path()).unwrap_or_else(|| "<unknown>".to_string())
            )));
        }

        if entries.len() >= MAX_MANIFEST_FILES {
            return Err(CapabilityError::refused(format!(
                "skill content exceeds the {MAX_MANIFEST_FILES}-file manifest limit"
            )));
        }

        let path = relative_path(root, entry.path()).ok_or_else(|| {
            CapabilityError::internal("skill file is not inside its own directory")
        })?;
        if is_marker_path(&path) {
            continue;
        }
        let bytes = std::fs::read(entry.path()).map_err(|error| {
            CapabilityError::internal(format!("failed to read skill file '{path}': {error}"))
        })?;
        total_bytes = total_bytes.saturating_add(bytes.len() as u64);
        if total_bytes > MAX_MANIFEST_BYTES {
            return Err(CapabilityError::refused(format!(
                "skill content exceeds the {MAX_MANIFEST_BYTES}-byte manifest limit"
            )));
        }

        entries.push(SkillFileEntry {
            path,
            sha256: hex::encode(Sha256::digest(&bytes)),
            size_bytes: bytes.len() as i64,
        });
    }

    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

/// Compares a directory against a recorded manifest.
pub fn verify_file_manifest(
    root: &Path,
    expected: &[SkillFileEntry],
) -> Result<ManifestVerdict, CapabilityError> {
    if !root.exists() {
        return Ok(ManifestVerdict::Drifted {
            missing: expected
                .iter()
                .map(|entry| entry.path.clone())
                .collect(),
            added: Vec::new(),
            changed: Vec::new(),
        });
    }

    let current = compute_file_manifest(root)?;
    let current_by_path: BTreeMap<&str, &SkillFileEntry> = current
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect();
    let expected_by_path: BTreeMap<&str, &SkillFileEntry> = expected
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect();

    let mut missing = Vec::new();
    let mut changed = Vec::new();
    for (path, expected_entry) in &expected_by_path {
        match current_by_path.get(path) {
            None => missing.push((*path).to_string()),
            Some(actual) if actual.sha256 != expected_entry.sha256 => {
                changed.push((*path).to_string())
            }
            Some(_) => {}
        }
    }
    let added: Vec<String> = current_by_path
        .keys()
        .filter(|path| !expected_by_path.contains_key(*path))
        .map(|path| (*path).to_string())
        .collect();

    if missing.is_empty() && added.is_empty() && changed.is_empty() {
        Ok(ManifestVerdict::Match)
    } else {
        Ok(ManifestVerdict::Drifted {
            missing,
            added,
            changed,
        })
    }
}

/// A directory-relative path with `/` separators, or `None` when the path is
/// not inside the root.
fn relative_path(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            std::path::Component::Normal(value) => parts.push(value.to_string_lossy().to_string()),
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn manifest_matches_and_detects_drift() {
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("skill");
        fs::create_dir_all(root.join("references")).expect("create dirs");
        fs::write(root.join("SKILL.md"), "---\nname: demo\n---\nbody").expect("write skill");
        fs::write(root.join("references").join("guide.md"), "guide").expect("write reference");

        let manifest = compute_file_manifest(&root).expect("manifest");
        assert_eq!(manifest.len(), 2);
        assert_eq!(manifest[0].path, "SKILL.md");
        assert_eq!(manifest[1].path, "references/guide.md");
        assert!(verify_file_manifest(&root, &manifest)
            .expect("verify")
            .is_match());

        fs::write(root.join("SKILL.md"), "---\nname: demo\n---\nCHANGED").expect("edit skill");
        let verdict = verify_file_manifest(&root, &manifest).expect("verify");
        match verdict {
            ManifestVerdict::Drifted { changed, .. } => assert_eq!(changed, vec!["SKILL.md".to_string()]),
            ManifestVerdict::Match => panic!("edited content must be reported as drift"),
        }
    }

    #[test]
    fn a_symlink_inside_a_skill_directory_is_refused() {
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("skill");
        fs::create_dir_all(&root).expect("create dirs");
        fs::write(root.join("SKILL.md"), "body").expect("write skill");

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/passwd", root.join("escape.md")).expect("symlink");
            let error = compute_file_manifest(&root).expect_err("symlink must be refused");
            assert_eq!(error.code(), crate::capability::error::code::REFUSED);
        }
    }

    #[test]
    fn a_missing_directory_is_reported_as_full_drift() {
        let temp = TempDir::new().expect("temp dir");
        let root = temp.path().join("skill");
        fs::create_dir_all(&root).expect("create dirs");
        fs::write(root.join("SKILL.md"), "body").expect("write skill");
        let manifest = compute_file_manifest(&root).expect("manifest");

        let verdict = verify_file_manifest(&temp.path().join("gone"), &manifest).expect("verify");
        match verdict {
            ManifestVerdict::Drifted { missing, .. } => {
                assert_eq!(missing, vec!["SKILL.md".to_string()])
            }
            ManifestVerdict::Match => panic!("a missing directory must be drift"),
        }
    }
}
