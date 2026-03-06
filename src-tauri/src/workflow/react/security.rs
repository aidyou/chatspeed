use crate::workflow::react::error::WorkflowEngineError;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::{Component, Path, PathBuf};

pub struct PathGuard {
    allowed_roots: Vec<(PathBuf, Option<Gitignore>)>,
    primary_root: Option<PathBuf>,
}

impl PathGuard {
    pub fn new(allowed_roots: Vec<PathBuf>) -> Self {
        let canonical_roots: Vec<PathBuf> = allowed_roots
            .into_iter()
            .filter_map(|p| p.canonicalize().ok())
            .collect();

        let primary_root = canonical_roots.first().cloned();

        let mut roots_with_ignore = Vec::new();
        for root in canonical_roots {
            let mut builder = GitignoreBuilder::new(&root);

            // Hierarchical Scan: Find all .gitignore files in the tree
            for entry in walkdir::WalkDir::new(&root)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| {
                    let name = e.file_name().to_string_lossy();
                    name != ".git" && name != "node_modules"
                })
                .filter_map(|e| e.ok())
            {
                if entry.file_name() == ".gitignore" {
                    let _ = builder.add(entry.path());
                }
            }

            let gitignore = builder.build().ok();
            roots_with_ignore.push((root, gitignore));
        }

        Self {
            allowed_roots: roots_with_ignore,
            primary_root,
        }
    }

    pub fn allowed_roots(&self) -> Vec<PathBuf> {
        self.allowed_roots.iter().map(|(r, _)| r.clone()).collect()
    }

    /// Checks if the normalized path is exactly one of the authorized roots.
    pub fn is_authorized_root(&self, path: &Path) -> bool {
        self.allowed_roots.iter().any(|(root, _)| root == path)
    }

    pub fn update_allowed_roots(&mut self, allowed_roots: Vec<PathBuf>) {
        let new_self = Self::new(allowed_roots);
        self.allowed_roots = new_self.allowed_roots;
        self.primary_root = new_self.primary_root;
    }

    /// Safely normalizes a path by resolving all '..' and '.' components without hitting the disk.
    fn normalize_path(path: &Path) -> PathBuf {
        let mut components = path.components().peekable();
        let mut ret = if let Some(c @ Component::Prefix(..)) = components.peek() {
            let c = c.clone();
            components.next();
            PathBuf::from(c.as_os_str())
        } else {
            PathBuf::new()
        };

        for component in components {
            match component {
                Component::Prefix(..) => unreachable!(),
                Component::RootDir => {
                    ret.push(component.as_os_str());
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    ret.pop();
                }
                Component::Normal(c) => {
                    ret.push(c);
                }
            }
        }
        ret
    }

    /// Validates if the target path is within the authorized workspace and NOT ignored by .gitignore.
    /// If restrict_to_planning is true, only allows access to specific directories (skills, tmp, planning).
    pub fn validate(&self, target: &Path, restrict_to_planning: bool) -> Result<PathBuf, WorkflowEngineError> {
        // 1. Resolve logical base for relative paths
        let abs_path = if target.is_absolute() {
            target.to_path_buf()
        } else {
            match &self.primary_root {
                Some(root) => root.join(target),
                None => return Err(WorkflowEngineError::Security(format!(
                    "Relative Path Denied: {:?} - No primary workspace authorized. Please set a workspace first.",
                    target
                ))),
            }
        };

        // 2. Normalize: Resolve all '..' and '.' components
        let normalized_path = Self::normalize_path(&abs_path);

        // 3. Planning Mode Restrictions
        if restrict_to_planning {
            let is_allowed_planning_dir = self.allowed_roots.iter().any(|(root, _)| {
                let folder_name = root.file_name().and_then(|n| n.to_str()).unwrap_or("");
                // Precise check: must be exactly in one of the safe roots
                let is_safe_root = ["skills", "tmp", "planning"].contains(&folder_name);
                is_safe_root && normalized_path.starts_with(root)
            });

            if !is_allowed_planning_dir {
                return Err(WorkflowEngineError::Security(format!(
                    "Planning Mode Restriction: {:?} is not in allowed planning directories (skills, tmp, planning)",
                    target
                )));
            }
        }

        // 4. Boundary and Gitignore Check
        for (root, gitignore) in &self.allowed_roots {
            if normalized_path.starts_with(root) {
                // Check gitignore if it exists for this root
                if let Some(gi) = gitignore {
                    if let Ok(rel_path) = normalized_path.strip_prefix(root) {
                        let mut current_p = PathBuf::new();
                        let components: Vec<_> = rel_path.components().collect();

                        for (i, comp) in components.iter().enumerate() {
                            if let Component::Normal(name) = comp {
                                current_p.push(name);
                                let is_dir = if i < components.len() - 1 {
                                    true 
                                } else {
                                    normalized_path.is_dir()
                                };

                                if gi.matched(&current_p, is_dir).is_ignore() {
                                    return Err(WorkflowEngineError::Security(format!(
                                        "Path Access Denied: {:?} is ignored by .gitignore in {:?}",
                                        target, root
                                    )));
                                }
                            }
                        }
                    }
                }
                return Ok(normalized_path);
            }
        }

        Err(WorkflowEngineError::Security(format!(
            "Path Access Denied: {:?} (normalized as {:?}) is outside allowed roots",
            target, normalized_path
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_path_guard_complex() {
        let root = tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let guard = PathGuard::new(vec![root_path.clone()]);

        // 1. Valid path
        let file1 = root_path.join("exists.txt");
        assert!(guard.validate(&file1, false).is_ok());

        // 2. Traversal attempt
        let traversal = root_path.join("../outside.txt");
        assert!(guard.validate(&traversal, false).is_err());

        // 3. Tricky nested traversal
        let tricky = root_path.join("sub/../../outside.txt");
        assert!(guard.validate(&tricky, false).is_err());
    }

    #[test]
    fn test_path_guard_gitignore() {
        let root = tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();

        // Create .gitignore
        fs::write(root_path.join(".gitignore"), "ignored.txt\nsecret_dir/").unwrap();
        fs::create_dir(root_path.join("secret_dir")).unwrap();
        fs::write(root_path.join("secret_dir/data.txt"), "secret").unwrap();
        fs::write(root_path.join("ignored.txt"), "ignored").unwrap();
        fs::write(root_path.join("allowed.txt"), "allowed").unwrap();

        let guard = PathGuard::new(vec![root_path.clone()]);

        // 1. Allowed file
        assert!(guard.validate(&root_path.join("allowed.txt"), false).is_ok());

        // 2. Ignored file
        assert!(guard.validate(&root_path.join("ignored.txt"), false).is_err());

        // 3. File in ignored directory
        assert!(guard
            .validate(&root_path.join("secret_dir/data.txt"), false)
            .is_err());

        // 4. The ignored directory itself
        assert!(guard.validate(&root_path.join("secret_dir"), false).is_err());
    }

    #[test]
    fn test_path_guard_empty_roots() {
        // Empty allowed roots should reject any path
        let guard = PathGuard::new(vec![]);
        let temp = tempdir().unwrap();
        let test_path = temp.path().join("any.txt");
        assert!(guard.validate(&test_path, false).is_err());
        // Relative path also denied
        assert!(guard.validate(Path::new("./any.txt"), false).is_err());
        // Absolute path denied
        assert!(guard.validate(Path::new("/tmp/any.txt"), false).is_err());
    }

    #[test]
    fn test_path_guard_multiple_roots() {
        let root1 = tempdir().unwrap();
        let root1_path = root1.path().canonicalize().unwrap();
        let root2 = tempdir().unwrap();
        let root2_path = root2.path().canonicalize().unwrap();
        let guard = PathGuard::new(vec![root1_path.clone(), root2_path.clone()]);

        // Path inside root1 allowed
        let file1 = root1_path.join("inside1.txt");
        assert!(guard.validate(&file1, false).is_ok());

        // Path inside root2 allowed
        let file2 = root2_path.join("inside2.txt");
        assert!(guard.validate(&file2, false).is_ok());

        // Path outside both roots denied
        let temp = tempdir().unwrap();
        let outside = temp.path().canonicalize().unwrap().join("outside.txt");
        assert!(guard.validate(&outside, false).is_err());
    }

    #[test]
    fn test_path_guard_normalize_edge_cases() {
        // Test normalization function directly
        assert_eq!(
            PathGuard::normalize_path(Path::new("/foo/bar/../baz")),
            PathBuf::from("/foo/baz")
        );
        assert_eq!(
            PathGuard::normalize_path(Path::new("/foo/./bar")),
            PathBuf::from("/foo/bar")
        );
        assert_eq!(
            PathGuard::normalize_path(Path::new("/foo//bar")),
            PathBuf::from("/foo/bar")
        );
        // Relative paths
        assert_eq!(
            PathGuard::normalize_path(Path::new("foo/../bar")),
            PathBuf::from("bar")
        );
        assert_eq!(
            PathGuard::normalize_path(Path::new("./foo")),
            PathBuf::from("foo")
        );
        // Empty path
        assert_eq!(PathGuard::normalize_path(Path::new("")), PathBuf::from(""));
        // Path ending with ..
        assert_eq!(
            PathGuard::normalize_path(Path::new("/foo/bar/..")),
            PathBuf::from("/foo")
        );
        // Path with multiple ..
        assert_eq!(
            PathGuard::normalize_path(Path::new("/foo/bar/../../baz")),
            PathBuf::from("/baz")
        );
    }

    #[test]
    fn test_path_guard_relative_to_cwd() {
        let root = tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let guard = PathGuard::new(vec![root_path.clone()]);

        // 1. Relative path from base (primary root) should be allowed
        let relative = Path::new("some_file.txt");
        assert!(guard.validate(relative, false).is_ok());

        // 2. Traversal outside root should be denied
        let traversal = Path::new("../outside.txt");
        assert!(guard.validate(traversal, false).is_err());
    }

    #[test]
    fn test_path_guard_complex_gitignore() {
        let root = tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();

        // Create .gitignore with wildcards, negations, and directory patterns
        fs::write(
            root_path.join(".gitignore"),
            "*.log\n\
            !important.log\n\
            build/\n\
            *.tmp\n\
            /root_ignore.txt\n\
            subdir/ignore_in_sub.txt\n",
        )
        .unwrap();

        // Create test files
        fs::write(root_path.join("app.log"), "log").unwrap();
        fs::write(root_path.join("important.log"), "important").unwrap();
        fs::write(root_path.join("other.tmp"), "tmp").unwrap();
        fs::write(root_path.join("root_ignore.txt"), "ignored").unwrap();
        fs::create_dir(root_path.join("build")).unwrap();
        fs::write(root_path.join("build/output.txt"), "output").unwrap();
        fs::create_dir(root_path.join("subdir")).unwrap();
        fs::write(root_path.join("subdir/ignore_in_sub.txt"), "ignored").unwrap();
        fs::write(root_path.join("subdir/allowed.txt"), "allowed").unwrap();

        let guard = PathGuard::new(vec![root_path.clone()]);

        // Wildcard denial
        assert!(guard.validate(&root_path.join("app.log"), false).is_err());
        // Negation allows specific file
        assert!(guard.validate(&root_path.join("important.log"), false).is_ok());
        // Another wildcard denial
        assert!(guard.validate(&root_path.join("other.tmp"), false).is_err());
        // Absolute path pattern
        assert!(guard.validate(&root_path.join("root_ignore.txt"), false).is_err());
        // Directory pattern denies entire directory
        assert!(guard.validate(&root_path.join("build/output.txt"), false).is_err());
        // Subdirectory specific pattern
        assert!(guard
            .validate(&root_path.join("subdir/ignore_in_sub.txt"), false)
            .is_err());
        // File not matching any pattern allowed
        assert!(guard
            .validate(&root_path.join("subdir/allowed.txt"), false)
            .is_ok());
    }

    #[test]
    fn test_path_guard_recursive_gitignore() {
        let root = tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();

        // 1. Create a root file (allowed)
        fs::write(root_path.join("root_file.txt"), "root").unwrap();

        // 2. Create a subdirectory with its own .gitignore
        let sub_dir = root_path.join("protected_sub");
        fs::create_dir(&sub_dir).unwrap();
        fs::write(sub_dir.join(".gitignore"), "secret_inner.txt\nprivate_dir/").unwrap();

        // 3. Create files inside the subdirectory
        fs::write(sub_dir.join("public.txt"), "public").unwrap();
        fs::write(sub_dir.join("secret_inner.txt"), "secret").unwrap();
        
        let private_dir = sub_dir.join("private_dir");
        fs::create_dir(&private_dir).unwrap();
        fs::write(private_dir.join("data.txt"), "private data").unwrap();

        // Create the guard (it should scan and find the nested .gitignore)
        let guard = PathGuard::new(vec![root_path.clone()]);

        // VERIFICATIONS:
        // Root file should be allowed
        assert!(guard.validate(&root_path.join("root_file.txt"), false).is_ok());
        
        // File in sub_dir NOT in its .gitignore should be allowed
        assert!(guard.validate(&sub_dir.join("public.txt"), false).is_ok());

        // File in sub_dir matching its local .gitignore should be DENIED
        assert!(guard.validate(&sub_dir.join("secret_inner.txt"), false).is_err());

        // Directory in sub_dir matching its local .gitignore should be DENIED
        assert!(guard.validate(&private_dir, false).is_err());

        // File inside a directory ignored by a local .gitignore should be DENIED
        assert!(guard.validate(&private_dir.join("data.txt"), false).is_err());
    }

    #[test]
    fn test_path_guard_error_messages() {
        let root = tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();

        // 1. Test Outside Root Error
        {
            let guard = PathGuard::new(vec![root_path.clone()]);
            let outside = Path::new("/some/outside/path");
            let err = guard.validate(outside, false).unwrap_err();
            match err {
                WorkflowEngineError::Security(msg) => {
                    assert!(msg.contains("Path Access Denied"));
                    assert!(msg.contains("outside allowed roots"));
                }
                _ => panic!("Expected Security error"),
            }
        }

        // 2. Test Gitignore Error
        {
            fs::write(root_path.join(".gitignore"), "secret.txt").unwrap();
            fs::write(root_path.join("secret.txt"), "secret").unwrap();
            
            // Re-create guard to load the new .gitignore
            let guard = PathGuard::new(vec![root_path.clone()]);
            let err = guard.validate(&root_path.join("secret.txt"), false).unwrap_err();
            match err {
                WorkflowEngineError::Security(msg) => {
                    assert!(msg.contains("Path Access Denied"));
                    assert!(msg.contains("ignored by .gitignore"));
                }
                _ => panic!("Expected Security error"),
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_path_guard_symlink() {
        // Test symlink handling
        use std::os::unix::fs::symlink;
        let root = tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let guard = PathGuard::new(vec![root_path.clone()]);

        // Create a symlink inside root pointing to a file outside
        let outside_dir = tempdir().unwrap();
        let outside_file = outside_dir.path().join("secret.txt");
        fs::write(&outside_file, "secret").unwrap();
        let symlink_path = root_path.join("link_to_secret.txt");
        symlink(&outside_file, &symlink_path).unwrap();

        // Symlink itself should be allowed (since it's inside root)
        // Note: PathGuard does not resolve symlinks, so it should allow access
        assert!(guard.validate(&symlink_path, false).is_ok());

        // The symlink target should be denied (outside root)
        assert!(guard.validate(&outside_file, false).is_err());
    }

    #[test]
    #[cfg(not(unix))]
    fn test_path_guard_symlink() {
        // Symlink test not applicable on non-Unix platforms
        // This test passes trivially
    }

    #[test]
    fn test_path_guard_planning_mode() {
        let root = tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        
        let skills_dir = root_path.join("skills");
        let tmp_dir = root_path.join("tmp");
        let planning_dir = root_path.join("planning");
        let other_dir = root_path.join("other");
        
        fs::create_dir(&skills_dir).unwrap();
        fs::create_dir(&tmp_dir).unwrap();
        fs::create_dir(&planning_dir).unwrap();
        fs::create_dir(&other_dir).unwrap();
        
        let guard = PathGuard::new(vec![
            skills_dir.clone(),
            tmp_dir.clone(),
            planning_dir.clone(),
            other_dir.clone()
        ]);
        
        // Allowed in planning mode
        assert!(guard.validate(&skills_dir.join("script.py"), true).is_ok());
        assert!(guard.validate(&tmp_dir.join("test.txt"), true).is_ok());
        assert!(guard.validate(&planning_dir.join("plan.md"), true).is_ok());
        
        // Denied in planning mode
        assert!(guard.validate(&other_dir.join("data.txt"), true).is_err());
        
        // Allowed in normal mode
        assert!(guard.validate(&other_dir.join("data.txt"), false).is_ok());
    }
}
