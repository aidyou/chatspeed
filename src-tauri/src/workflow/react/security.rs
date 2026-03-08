use crate::workflow::react::error::WorkflowEngineError;

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::{Component, Path, PathBuf};

/// Critical system directories that should NEVER be accessed by the AI
const SENSITIVE_SYSTEM_PATHS: &[&str] = &[
    "/etc",
    "/bin",
    "/sbin",
    "/usr/bin",
    "/usr/sbin",
    "/lib",
    "/usr/lib",
    "/lib64",
    "/var",
    "/root",
    "/boot",
    "/dev",
    "/sys",
    "/proc",
    "/usr/local/bin",
    "C:\\Windows",
    "C:\\System32",
    "C:\\Program Files",
    "C:\\Program Files (x86)",
];

pub struct PathGuard {
    workspace_roots: Vec<(PathBuf, Option<Gitignore>)>,
    sandbox_roots: Vec<(PathBuf, Option<Gitignore>)>,
    skill_roots: Vec<PathBuf>,
    primary_root: Option<PathBuf>,
}

impl PathGuard {
    pub fn new(
        workspace_paths: Vec<PathBuf>,
        sandbox_paths: Vec<PathBuf>,
        skill_paths: Vec<PathBuf>,
    ) -> Self {
        let workspace_roots = Self::process_roots(workspace_paths);
        let sandbox_roots = Self::process_roots(sandbox_paths);
        let skill_roots = skill_paths
            .into_iter()
            .filter_map(|p| p.canonicalize().ok().or_else(|| Some(p)))
            .collect();
        let primary_root = workspace_roots.first().map(|(p, _)| p.clone());

        Self {
            workspace_roots,
            sandbox_roots,
            skill_roots,
            primary_root,
        }
    }

    fn process_roots(paths: Vec<PathBuf>) -> Vec<(PathBuf, Option<Gitignore>)> {
        let canonical_roots: Vec<PathBuf> = paths
            .into_iter()
            .filter_map(|p| p.canonicalize().ok())
            .collect();
        let mut roots_with_ignore = Vec::new();
        for root in canonical_roots {
            let mut builder = GitignoreBuilder::new(&root);
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
        roots_with_ignore
    }

    pub fn allowed_roots(&self) -> Vec<PathBuf> {
        let mut all = Vec::new();
        for (r, _) in &self.workspace_roots {
            all.push(r.clone());
        }
        for (r, _) in &self.sandbox_roots {
            all.push(r.clone());
        }
        all
    }

    fn is_sensitive_path(path: &Path) -> bool {
        if path == Path::new("/") {
            return true;
        }
        SENSITIVE_SYSTEM_PATHS
            .iter()
            .any(|prefix| path.starts_with(prefix))
    }

    pub fn is_authorized_root(&self, path: &Path) -> bool {
        let physical_path = path
            .canonicalize()
            .unwrap_or_else(|_| Self::normalize_path(path));
        if Self::is_sensitive_path(&physical_path) {
            return false;
        }
        self.workspace_roots
            .iter()
            .any(|(root, _)| root == &physical_path)
            || self
                .sandbox_roots
                .iter()
                .any(|(root, _)| root == &physical_path)
    }

    pub fn is_within_skill_root(&self, path: &Path) -> bool {
        let physical_path = path
            .canonicalize()
            .unwrap_or_else(|_| Self::normalize_path(path));
        if Self::is_sensitive_path(&physical_path) {
            return false;
        }
        self.skill_roots
            .iter()
            .any(|root| physical_path.starts_with(root))
    }

    pub fn update_allowed_roots(&mut self, workspace_paths: Vec<PathBuf>) {
        let processed = Self::process_roots(workspace_paths);
        self.workspace_roots = processed;
        self.primary_root = self.workspace_roots.first().map(|(p, _)| p.clone());
    }

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

    pub fn validate(
        &self,
        target: &Path,
        is_planning_phase: bool,
        is_write: bool,
        is_delete: bool,
    ) -> Result<PathBuf, WorkflowEngineError> {
        let abs_path = if target.is_absolute() {
            target.to_path_buf()
        } else {
            match &self.primary_root {
                Some(root) => root.join(target),
                None => {
                    return Err(WorkflowEngineError::Security(format!(
                        "Relative Path Denied: {:?} - No primary workspace.",
                        target
                    )))
                }
            }
        };

        let final_path = abs_path
            .canonicalize()
            .unwrap_or_else(|_| Self::normalize_path(&abs_path));

        if Self::is_sensitive_path(&final_path) {
            return Err(WorkflowEngineError::Security(format!(
                "CRITICAL SECURITY BLOCK: Access to sensitive path {:?} is forbidden.",
                final_path
            )));
        }

        // 1. Check Sandbox (Always OK for everything)
        for (root, gitignore) in &self.sandbox_roots {
            if final_path.starts_with(root) {
                self.check_gitignore(root, gitignore, &final_path)?;
                return Ok(final_path);
            }
        }

        // 2. Check Skill Roots
        for root in &self.skill_roots {
            if final_path.starts_with(root) {
                // Skills: Allow Write, but BLOCKED for Delete (Requires Manual Review)
                if is_delete {
                    return Err(WorkflowEngineError::Security(format!(
                        "PERMISSION DENIED: Deleting files in skill directory {:?} requires manual review or explicit authorization.",
                        final_path
                    )));
                }
                return Ok(final_path);
            }
        }

        // 3. Check Workspace
        for (root, gitignore) in &self.workspace_roots {
            if final_path.starts_with(root) {
                if is_planning_phase && is_write {
                    return Err(WorkflowEngineError::Security(format!(
                        "Planning Mode Restriction: Write denied to workspace {:?}",
                        target
                    )));
                }
                // Global rule: Delete in workspace also restricted
                if is_delete {
                    return Err(WorkflowEngineError::Security(format!(
                        "PERMISSION DENIED: Deleting workspace files {:?} is forbidden.",
                        final_path
                    )));
                }
                self.check_gitignore(root, gitignore, &final_path)?;
                return Ok(final_path);
            }
        }

        Err(WorkflowEngineError::Security(format!(
            "Path Access Denied: {:?} is outside allowed roots",
            target
        )))
    }

    fn check_gitignore(
        &self,
        root: &Path,
        gitignore: &Option<Gitignore>,
        path: &Path,
    ) -> Result<(), WorkflowEngineError> {
        if let Some(gi) = gitignore {
            if let Ok(rel_path) = path.strip_prefix(root) {
                let mut current_p = PathBuf::new();
                let components: Vec<_> = rel_path.components().collect();
                for (i, comp) in components.iter().enumerate() {
                    if let Component::Normal(name) = comp {
                        current_p.push(name);
                        let is_dir = if i < components.len() - 1 {
                            true
                        } else {
                            path.is_dir()
                        };
                        if gi.matched(&current_p, is_dir).is_ignore() {
                            return Err(WorkflowEngineError::Security(format!(
                                "Path Denied: {:?} is ignored by .gitignore",
                                path
                            )));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_path_guard_skills_permissions() {
        let root = tempdir().unwrap();
        let skill_path = root.path().join("skills");
        fs::create_dir(&skill_path).unwrap();
        let skill_path = skill_path.canonicalize().unwrap();

        let guard = PathGuard::new(vec![], vec![], vec![skill_path.clone()]);

        // Skills: Write OK
        assert!(guard
            .validate(&skill_path.join("new_skill.py"), false, true, false)
            .is_ok());

        // Skills: Delete DENIED (Hard block in PathGuard to force review)
        assert!(guard
            .validate(&skill_path.join("old_skill.py"), false, true, true)
            .is_err());
    }

    #[test]
    fn test_path_guard_system_protection() {
        let ws = tempdir().unwrap();
        let ws_path = ws.path().canonicalize().unwrap();
        let guard = PathGuard::new(vec![ws_path.clone()], vec![], vec![]);
        assert!(guard
            .validate(&ws_path.join("file.txt"), false, false, false)
            .is_ok());
        assert!(guard
            .validate(Path::new("/etc/passwd"), false, false, false)
            .is_err());
    }

    #[test]
    fn test_path_guard_complex() {
        let root = tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let guard = PathGuard::new(vec![root_path.clone()], vec![], vec![]);
        assert!(guard
            .validate(&root_path.join("exists.txt"), false, false, false)
            .is_ok());
        assert!(guard
            .validate(&root_path.join("../outside.txt"), false, false, false)
            .is_err());
    }

    #[test]
    fn test_path_guard_gitignore() {
        let root = tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        fs::write(root_path.join(".gitignore"), "ignored.txt").unwrap();
        let guard = PathGuard::new(vec![root_path.clone()], vec![], vec![]);
        assert!(guard
            .validate(&root_path.join("ignored.txt"), false, false, false)
            .is_err());
    }

    #[test]
    fn test_path_guard_skill_root() {
        let root = tempdir().unwrap();
        let skill_path = root.path().join("my-skills");
        fs::create_dir(&skill_path).unwrap();
        let skill_path = skill_path.canonicalize().unwrap();
        let guard = PathGuard::new(vec![], vec![], vec![skill_path.clone()]);
        assert!(guard.is_within_skill_root(&skill_path.join("test.py")));
    }

    #[test]
    fn test_path_guard_empty_roots() {
        let guard = PathGuard::new(vec![], vec![], vec![]);
        assert!(guard
            .validate(Path::new("/tmp/any.txt"), false, false, false)
            .is_err());
    }

    #[test]
    fn test_path_guard_normalize_edge_cases() {
        assert_eq!(
            PathGuard::normalize_path(Path::new("/a/b/../c")),
            PathBuf::from("/a/c")
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_path_guard_symlink_security() {
        let ws = tempdir().unwrap();
        let ws_path = ws.path().canonicalize().unwrap();
        let secret = tempdir().unwrap();
        let secret_file = secret.path().join("s.txt");
        fs::write(&secret_file, "shh").unwrap();
        use std::os::unix::fs::symlink;
        let link = ws_path.join("l");
        symlink(&secret_file, &link).unwrap();
        let guard = PathGuard::new(vec![ws_path.clone()], vec![], vec![]);
        assert!(guard.validate(&link, false, false, false).is_err());
    }

    #[test]
    fn test_path_guard_planning_mode() {
        let root = tempdir().unwrap();
        let ws = root.path().join("ws");
        let sb = root.path().join("sb");
        fs::create_dir(&ws).unwrap();
        fs::create_dir(&sb).unwrap();
        let guard = PathGuard::new(
            vec![ws.canonicalize().unwrap()],
            vec![sb.canonicalize().unwrap()],
            vec![],
        );
        assert!(guard
            .validate(&ws.join("f.txt"), true, false, false)
            .is_ok());
        assert!(guard
            .validate(&ws.join("f.txt"), true, true, false)
            .is_err());
        assert!(guard.validate(&sb.join("f.txt"), true, true, false).is_ok());
    }
}
