use std::path::{Path, PathBuf};
use crate::workflow::react::error::WorkflowEngineError;

pub struct PathGuard {
    allowed_roots: Vec<PathBuf>,
}

impl PathGuard {
    pub fn new(allowed_roots: Vec<PathBuf>) -> Self {
        let canonical_roots = allowed_roots.into_iter()
            .filter_map(|p| p.canonicalize().ok())
            .collect();
        Self { allowed_roots: canonical_roots }
    }

    pub fn allowed_roots(&self) -> &[PathBuf] {
        &self.allowed_roots
    }

    /// Validates if the target path is within the authorized workspace.
    /// Returns the canonicalized path if valid.
    pub fn validate(&self, target: &Path) -> Result<PathBuf, WorkflowEngineError> {
        // We use absolute() or canonicalize() to prevent directory traversal attacks
        // Note: target might not exist yet if we are creating it, so we handle parent
        let target_abs = if target.exists() {
            target.canonicalize()
                .map_err(|e| WorkflowEngineError::Security(format!("Failed to canonicalize path: {}", e)))?
        } else {
            // If target doesn't exist, validate its parent
            let parent = target.parent().ok_or_else(|| WorkflowEngineError::Security("Path has no parent".to_string()))?;
            let parent_abs = parent.canonicalize()
                .map_err(|e| WorkflowEngineError::Security(format!("Failed to canonicalize parent path: {}", e)))?;
            parent_abs.join(target.file_name().ok_or_else(|| WorkflowEngineError::Security("Invalid filename".to_string()))?)
        };

        let is_allowed = self.allowed_roots.iter().any(|root| {
            target_abs.starts_with(root)
        });

        if is_allowed {
            Ok(target_abs)
        } else {
            Err(WorkflowEngineError::Security(format!(
                "Path {:?} is outside the authorized workspace. Authorized roots: {:?}",
                target, self.allowed_roots
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_path_guard_basic() {
        let dir1 = tempdir().unwrap();
        let dir2 = tempdir().unwrap();
        let outside = tempdir().unwrap();

        let guard = PathGuard::new(vec![dir1.path().to_path_buf(), dir2.path().to_path_buf()]);

        // Valid inside dir1
        let file1 = dir1.path().join("test.txt");
        fs::write(&file1, "hello").unwrap();
        assert!(guard.validate(&file1).is_ok());

        // Valid inside dir2
        let file2 = dir2.path().join("sub/test.txt");
        fs::create_dir_all(file2.parent().unwrap()).unwrap();
        fs::write(&file2, "hello").unwrap();
        assert!(guard.validate(&file2).is_ok());

        // Invalid outside
        let file_outside = outside.path().join("dangerous.txt");
        fs::write(&file_outside, "boom").unwrap();
        assert!(guard.validate(&file_outside).is_err());
    }

    #[test]
    fn test_path_guard_traversal() {
        let dir = tempdir().unwrap();
        let guard = PathGuard::new(vec![dir.path().to_path_buf()]);

        // Attempt traversal: dir/../outside
        let traversal = dir.path().join("../outside.txt");
        // validate will canonicalize it, and it will be outside
        assert!(guard.validate(&traversal).is_err());
    }
}
