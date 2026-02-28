use crate::workflow::react::error::WorkflowEngineError;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::{Component, Path, PathBuf};

pub struct PathGuard {
    allowed_roots: Vec<(PathBuf, Option<Gitignore>)>,
}

impl PathGuard {
    pub fn new(allowed_roots: Vec<PathBuf>) -> Self {
        let canonical_roots: Vec<PathBuf> = allowed_roots
            .into_iter()
            .filter_map(|p| p.canonicalize().ok())
            .collect();

        let mut roots_with_ignore = Vec::new();
        for root in canonical_roots {
            let mut builder = GitignoreBuilder::new(&root);

            // Hierarchical Scan: Find all .gitignore files in the tree
            // We skip .git and node_modules to keep initialization fast.
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
                    if let Some(err) = builder.add(entry.path()) {
                        log::warn!(
                            "Error loading nested gitignore from {:?}: {}",
                            entry.path(),
                            err
                        );
                    }
                }
            }

            let gitignore = builder.build().ok();
            roots_with_ignore.push((root, gitignore));
        }

        Self {
            allowed_roots: roots_with_ignore,
        }
    }

    pub fn allowed_roots(&self) -> Vec<PathBuf> {
        self.allowed_roots.iter().map(|(r, _)| r.clone()).collect()
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
    pub fn validate(&self, target: &Path) -> Result<PathBuf, WorkflowEngineError> {
        // 1. Get the current working directory for relative path resolution
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));

        // 2. Make the path absolute (if it isn't)
        let abs_path = if target.is_absolute() {
            target.to_path_buf()
        } else {
            cwd.join(target)
        };

        // 3. Normalize: Resolve all '..' and '.' components
        let normalized_path = Self::normalize_path(&abs_path);

        // 4. Boundary and Gitignore Check
        for (root, gitignore) in &self.allowed_roots {
            if normalized_path.starts_with(root) {
                // Check gitignore if it exists for this root
                if let Some(gi) = gitignore {
                    if let Ok(rel_path) = normalized_path.strip_prefix(root) {
                        // The ignore crate's `matched` method doesn't automatically check parents
                        // for directory-based ignores like `node_modules/`.
                        // We must check the path itself and all its ancestor components relative to the root.

                        let mut current_p = PathBuf::new();
                        let components: Vec<_> = rel_path.components().collect();

                        for (i, comp) in components.iter().enumerate() {
                            if let Component::Normal(name) = comp {
                                current_p.push(name);
                                let is_dir = if i < components.len() - 1 {
                                    true // It's a parent component, so it's a directory
                                } else {
                                    normalized_path.is_dir() // Use actual metadata if it's the leaf
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
                // If it starts with an allowed root and is NOT ignored, it's valid
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
        assert!(guard.validate(&file1).is_ok());

        // 2. Traversal attempt
        let traversal = root_path.join("../outside.txt");
        assert!(guard.validate(&traversal).is_err());

        // 3. Tricky nested traversal
        let tricky = root_path.join("sub/../../outside.txt");
        assert!(guard.validate(&tricky).is_err());
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
        assert!(guard.validate(&root_path.join("allowed.txt")).is_ok());

        // 2. Ignored file
        assert!(guard.validate(&root_path.join("ignored.txt")).is_err());

        // 3. File in ignored directory
        assert!(guard
            .validate(&root_path.join("secret_dir/data.txt"))
            .is_err());

        // 4. The ignored directory itself
        assert!(guard.validate(&root_path.join("secret_dir")).is_err());
    }

    #[test]
    fn test_path_guard_empty_roots() {
        // Empty allowed roots should reject any path
        let guard = PathGuard::new(vec![]);
        let temp = tempdir().unwrap();
        let test_path = temp.path().join("any.txt");
        assert!(guard.validate(&test_path).is_err());
        // Relative path also denied
        assert!(guard.validate(Path::new("./any.txt")).is_err());
        // Absolute path denied
        assert!(guard.validate(Path::new("/tmp/any.txt")).is_err());
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
        assert!(guard.validate(&file1).is_ok());

        // Path inside root2 allowed
        let file2 = root2_path.join("inside2.txt");
        assert!(guard.validate(&file2).is_ok());

        // Path outside both roots denied
        let temp = tempdir().unwrap();
        let outside = temp.path().canonicalize().unwrap().join("outside.txt");
        assert!(guard.validate(&outside).is_err());
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

        // Change current directory to the root
        let original_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&root_path).unwrap();

        // Relative path from CWD should be allowed
        let relative = Path::new("some_file.txt");
        assert!(guard.validate(relative).is_ok());

        // Path outside root via relative traversal should be denied
        let traversal = Path::new("../outside.txt");
        assert!(guard.validate(traversal).is_err());

        // Restore original CWD
        std::env::set_current_dir(&original_cwd).unwrap();
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
        assert!(guard.validate(&root_path.join("app.log")).is_err());
        // Negation allows specific file
        assert!(guard.validate(&root_path.join("important.log")).is_ok());
        // Another wildcard denial
        assert!(guard.validate(&root_path.join("other.tmp")).is_err());
        // Absolute path pattern
        assert!(guard.validate(&root_path.join("root_ignore.txt")).is_err());
        // Directory pattern denies entire directory
        assert!(guard.validate(&root_path.join("build/output.txt")).is_err());
        // Subdirectory specific pattern
        assert!(guard
            .validate(&root_path.join("subdir/ignore_in_sub.txt"))
            .is_err());
        // File not matching any pattern allowed
        assert!(guard
            .validate(&root_path.join("subdir/allowed.txt"))
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
        assert!(guard.validate(&root_path.join("root_file.txt")).is_ok());
        
        // File in sub_dir NOT in its .gitignore should be allowed
        assert!(guard.validate(&sub_dir.join("public.txt")).is_ok());

        // File in sub_dir matching its local .gitignore should be DENIED
        assert!(guard.validate(&sub_dir.join("secret_inner.txt")).is_err());

        // Directory in sub_dir matching its local .gitignore should be DENIED
        assert!(guard.validate(&private_dir).is_err());

        // File inside a directory ignored by a local .gitignore should be DENIED
        assert!(guard.validate(&private_dir.join("data.txt")).is_err());
    }

    #[test]
    fn test_path_guard_error_messages() {
        let root = tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();

        // 1. Test Outside Root Error
        {
            let guard = PathGuard::new(vec![root_path.clone()]);
            let outside = Path::new("/some/outside/path");
            let err = guard.validate(outside).unwrap_err();
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
            let err = guard.validate(&root_path.join("secret.txt")).unwrap_err();
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
        assert!(guard.validate(&symlink_path).is_ok());

        // The symlink target should be denied (outside root)
        assert!(guard.validate(&outside_file).is_err());
    }

    #[test]
    #[cfg(not(unix))]
    fn test_path_guard_symlink() {
        // Symlink test not applicable on non-Unix platforms
        // This test passes trivially
    }
}
