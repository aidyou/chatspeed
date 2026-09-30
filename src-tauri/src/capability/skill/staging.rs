//! Private staging for Skill sources.
//!
//! Untrusted content is never analyzed in place: an archive or a remote source
//! is materialized under a per-operation directory in app data, checked there,
//! and deleted afterwards. Staging is not a durability domain — the journal is
//! (AC-2) — so a crash leaving residue is safe and is reported by doctor.

use std::path::{Path, PathBuf};

use crate::capability::error::CapabilityError;

/// One private staging directory for a single operation.
pub struct StagingArea {
    root: PathBuf,
    persistent: bool,
}

impl StagingArea {
    /// Creates a fresh staging directory for one operation.
    ///
    /// The directory is created private to the user; an already existing
    /// directory for the same operation id is refused so two concurrent
    /// attempts can never share one staging tree.
    pub fn create(app_data_dir: &Path, operation_id: &str) -> Result<Self, CapabilityError> {
        let parent = crate::capability::staging_dir(app_data_dir);
        std::fs::create_dir_all(&parent).map_err(|error| {
            CapabilityError::internal(format!("failed to create the staging root: {error}"))
        })?;
        restrict_permissions(&parent)?;

        let root = parent.join(sanitize_component(operation_id));
        if root.exists() {
            return Err(CapabilityError::busy(format!(
                "a staging directory for operation '{operation_id}' already exists"
            )));
        }
        std::fs::create_dir_all(&root).map_err(|error| {
            CapabilityError::internal(format!("failed to create a staging directory: {error}"))
        })?;
        restrict_permissions(&root)?;
        Ok(Self {
            root,
            persistent: false,
        })
    }

    /// The staging root for this operation.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Maps an operation id to the sanitized staging directory name it uses.
    ///
    /// Reconcile derives the same name to tell a live operation's staging tree
    /// from orphaned residue without a repository query (staging is not a
    /// durability domain, so residue is safe to remove once no operation owns it).
    pub fn sanitize_operation(value: &str) -> String {
        sanitize_component(value)
    }

    /// Keeps the directory for post-mortem inspection (doctor will report it).
    pub fn keep(mut self) {
        self.persistent = true;
    }

    /// Removes the staging tree, ignoring an already-missing directory.
    pub fn cleanup(&self) -> Result<(), CapabilityError> {
        if self.persistent || !self.root.exists() {
            return Ok(());
        }
        std::fs::remove_dir_all(&self.root).map_err(|error| {
            CapabilityError::internal(format!("failed to remove the staging directory: {error}"))
        })
    }
}

impl Drop for StagingArea {
    fn drop(&mut self) {
        // Staging is short-lived by contract: a caller that forgets to clean up
        // must not leak an untrusted tree into app data.
        if !self.persistent {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

/// Restricts a directory to the current user on Unix.
pub fn restrict_permissions(path: &Path) -> Result<(), CapabilityError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(0o700);
        std::fs::set_permissions(path, permissions).map_err(|error| {
            CapabilityError::internal(format!("failed to restrict directory permissions: {error}"))
        })?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Keeps only characters that are safe in a single path component.
fn sanitize_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .take(64)
        .collect();
    if sanitized.is_empty() {
        "operation".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn a_staging_area_is_isolated_and_removed_on_drop() {
        let temp = TempDir::new().expect("temp dir");
        let app_data = temp.path().join("app-data");
        let area = StagingArea::create(&app_data, "op-skill-1").expect("staging");
        let root = area.root().to_path_buf();
        assert!(root.is_dir());
        std::fs::write(root.join("content"), b"data").expect("write");
        drop(area);
        assert!(!root.exists(), "staging must not leak after drop");
    }

    #[test]
    fn two_attempts_never_share_one_staging_directory() {
        let temp = TempDir::new().expect("temp dir");
        let app_data = temp.path().join("app-data");
        let first = StagingArea::create(&app_data, "op-skill-1").expect("first staging");
        let error = StagingArea::create(&app_data, "op-skill-1")
            .err()
            .expect("a second attempt must be refused");
        assert_eq!(error.code(), crate::capability::error::code::BUSY);
        first.cleanup().expect("cleanup");
    }

    #[test]
    fn an_operation_id_cannot_escape_the_staging_root() {
        let temp = TempDir::new().expect("temp dir");
        let app_data = temp.path().join("app-data");
        let area = StagingArea::create(&app_data, "../../etc").expect("staging");
        assert!(area.root().starts_with(crate::capability::staging_dir(&app_data)));
        assert_eq!(area.root().file_name().unwrap(), "etc");
    }
}
