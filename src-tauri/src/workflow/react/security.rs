use crate::libs::ai_temp::resolve_ai_temp_path;
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

pub const CHATSPEED_IGNORE_FILE: &str = ".csignore";

pub fn is_workspace_noise_name(name: &str) -> bool {
    let name_lower = name.to_ascii_lowercase();
    matches!(
        name,
        ".git"
            | ".svn"
            | ".hg"
            | "node_modules"
            | "target"
            | "__pycache__"
            | ".venv"
            | ".turbo"
            | ".next"
            | ".nuxt"
            | ".svelte-kit"
            | "dist"
            | "coverage"
    ) || name_lower.ends_with(".pyc")
        || matches!(
            name_lower.as_str(),
            ".ds_store" | "thumbs.db" | "desktop.ini"
        )
}

pub fn workspace_walk_builder(path: &Path) -> ignore::WalkBuilder {
    let mut builder = ignore::WalkBuilder::new(path);
    builder
        .standard_filters(true)
        .hidden(false)
        .add_custom_ignore_filename(CHATSPEED_IGNORE_FILE);
    builder.filter_entry(|entry| {
        entry.depth() == 0
            || entry
                .file_name()
                .to_str()
                .is_none_or(|name| !is_workspace_noise_name(name))
    });
    builder
}

struct IgnoreScope {
    base_dir: PathBuf,
    matcher: Gitignore,
}

struct AuthorizedRoot {
    path: PathBuf,
    gitignore_scopes: Vec<IgnoreScope>,
    chatspeed_scopes: Vec<IgnoreScope>,
}

pub struct PathGuard {
    workspace_roots: Vec<AuthorizedRoot>,
    sandbox_roots: Vec<AuthorizedRoot>,
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
        let primary_root = workspace_roots.first().map(|root| root.path.clone());

        Self {
            workspace_roots,
            sandbox_roots,
            skill_roots,
            primary_root,
        }
    }

    fn process_roots(paths: Vec<PathBuf>) -> Vec<AuthorizedRoot> {
        let mut roots_with_ignore = Vec::new();
        for p in paths {
            // Try to canonicalize, but fallback to original if it fails but exists
            let root = match p.canonicalize() {
                Ok(canonical) => canonical,
                Err(_) => {
                    if p.exists() {
                        p
                    } else {
                        log::info!("[PathGuard] Path does not exist and was ignored: {:?}", p);
                        continue;
                    }
                }
            };

            let gitignore_scopes = Self::collect_ignore_scopes(&root, ".gitignore");
            let chatspeed_scopes = Self::collect_ignore_scopes(&root, CHATSPEED_IGNORE_FILE);
            roots_with_ignore.push(AuthorizedRoot {
                path: root,
                gitignore_scopes,
                chatspeed_scopes,
            });
        }
        roots_with_ignore
    }

    fn collect_ignore_scopes(root: &Path, file_name: &str) -> Vec<IgnoreScope> {
        let mut scopes = Vec::new();
        // Scan up to 3 levels deep to avoid massive stalls on large workspaces.
        for entry in walkdir::WalkDir::new(root)
            .max_depth(3)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_none_or(|name| !is_workspace_noise_name(name))
            })
            .filter_map(|e| e.ok())
        {
            if entry.file_name() == file_name {
                let Some(base_dir) = entry.path().parent().map(Path::to_path_buf) else {
                    continue;
                };
                let mut builder = GitignoreBuilder::new(&base_dir);
                if builder.add(entry.path()).is_some() {
                    continue;
                }
                if let Ok(matcher) = builder.build() {
                    scopes.push(IgnoreScope { base_dir, matcher });
                }
            }
        }
        scopes.sort_by(|a, b| {
            b.base_dir
                .components()
                .count()
                .cmp(&a.base_dir.components().count())
        });
        scopes
    }

    pub fn workspace_roots(&self) -> Vec<PathBuf> {
        self.workspace_roots
            .iter()
            .map(|root| root.path.clone())
            .collect()
    }

    pub fn get_primary_root(&self) -> Option<&std::path::Path> {
        self.primary_root.as_deref()
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
        let requested_path = Self::normalize_requested_path(path);
        let physical_path = requested_path
            .canonicalize()
            .unwrap_or_else(|_| Self::normalize_path(&requested_path));
        if Self::is_sensitive_path(&physical_path) {
            return false;
        }
        self.workspace_roots
            .iter()
            .any(|root| root.path == physical_path)
            || self
                .sandbox_roots
                .iter()
                .any(|root| root.path == physical_path)
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
        self.primary_root = self.workspace_roots.first().map(|root| root.path.clone());
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

    fn normalize_requested_path(path: &Path) -> PathBuf {
        let normalized = Self::normalize_path(path);
        resolve_ai_temp_path(&normalized)
    }

    fn resolve_physical_path(path: &Path) -> PathBuf {
        if let Ok(canonical) = path.canonicalize() {
            return canonical;
        }

        let mut missing_components = Vec::new();
        let mut current = path;
        while let Some(parent) = current.parent() {
            if let Some(name) = current.file_name() {
                missing_components.push(name.to_os_string());
            }
            if let Ok(mut canonical_parent) = parent.canonicalize() {
                for component in missing_components.iter().rev() {
                    canonical_parent.push(component);
                }
                return Self::normalize_path(&canonical_parent);
            }
            current = parent;
        }

        Self::normalize_path(path)
    }

    pub fn validate(
        &self,
        target: &Path,
        is_planning_phase: bool,
        is_write: bool,
        is_delete: bool,
    ) -> Result<PathBuf, WorkflowEngineError> {
        self.validate_with_ignore(target, is_planning_phase, is_write, is_delete)
    }

    pub fn validate_for_listing(
        &self,
        target: &Path,
        is_planning_phase: bool,
    ) -> Result<PathBuf, WorkflowEngineError> {
        self.validate_with_ignore(target, is_planning_phase, false, false)
    }

    pub fn is_chatspeed_allowed(
        &self,
        target: &Path,
        _is_dir: bool,
    ) -> Result<bool, WorkflowEngineError> {
        let resolved_target = resolve_ai_temp_path(target);
        let abs_path = if resolved_target.is_absolute() {
            resolved_target
        } else {
            match &self.primary_root {
                Some(root) => root.join(resolved_target),
                None => return Ok(false),
            }
        };
        let requested_path = Self::normalize_requested_path(&abs_path);
        let final_path = Self::resolve_physical_path(&requested_path);

        for root in self.sandbox_roots.iter().chain(self.workspace_roots.iter()) {
            if final_path.starts_with(&root.path) {
                self.check_ignore_rules(root, &final_path)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn validate_with_ignore(
        &self,
        target: &Path,
        is_planning_phase: bool,
        is_write: bool,
        is_delete: bool,
    ) -> Result<PathBuf, WorkflowEngineError> {
        let resolved_target = resolve_ai_temp_path(target);
        let abs_path = if resolved_target.is_absolute() {
            resolved_target
        } else {
            match &self.primary_root {
                Some(root) => root.join(resolved_target),
                None => {
                    return Err(WorkflowEngineError::Security(format!(
                        "Relative Path Denied: {:?} - No primary workspace is configured. You MUST provide an absolute path, or ask the user to add the directory to 'Authorized Paths' in settings.",
                        target
                    )))
                }
            }
        };

        let requested_path = Self::normalize_requested_path(&abs_path);
        let final_path = Self::resolve_physical_path(&requested_path);

        if Self::is_sensitive_path(&final_path) {
            return Err(WorkflowEngineError::Security(format!(
                "CRITICAL SECURITY BLOCK: Access to sensitive path {:?} is forbidden.",
                final_path
            )));
        }

        // 1. Check Sandbox (Always OK for everything)
        for root in &self.sandbox_roots {
            if final_path.starts_with(&root.path) {
                self.check_ignore_rules(root, &final_path)?;
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
        for root in &self.workspace_roots {
            if final_path.starts_with(&root.path) {
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
                self.check_ignore_rules(root, &final_path)?;
                return Ok(final_path);
            }
        }

        Err(WorkflowEngineError::Security(format!(
            "Path Access Denied: {:?} is outside allowed roots",
            target
        )))
    }

    fn check_ignore_scope(
        &self,
        scopes: &[IgnoreScope],
        path: &Path,
        is_dir_override: Option<bool>,
        denied_by: &str,
    ) -> Result<bool, WorkflowEngineError> {
        for scope in scopes {
            if !path.starts_with(&scope.base_dir) {
                continue;
            }
            if let Ok(rel_path) = path.strip_prefix(&scope.base_dir) {
                let mut current_p = PathBuf::new();
                let components: Vec<_> = rel_path.components().collect();
                for (i, comp) in components.iter().enumerate() {
                    if let Component::Normal(name) = comp {
                        current_p.push(name);
                        let is_dir = if i < components.len() - 1 {
                            true
                        } else {
                            is_dir_override.unwrap_or_else(|| path.is_dir())
                        };
                        let matched = scope.matcher.matched(&current_p, is_dir);
                        if matched.is_ignore() {
                            return Err(WorkflowEngineError::Security(format!(
                                "Path Denied: {:?} is ignored by {}",
                                path, denied_by
                            )));
                        }
                        if matched.is_whitelist() {
                            return Ok(true);
                        }
                    }
                }
            }
        }
        Ok(false)
    }

    fn check_gitignore(
        &self,
        root: &AuthorizedRoot,
        path: &Path,
    ) -> Result<(), WorkflowEngineError> {
        self.check_ignore_scope(&root.gitignore_scopes, path, None, ".gitignore")
            .map(|_| ())
    }

    fn check_chatspeed_ignore(
        &self,
        root: &AuthorizedRoot,
        path: &Path,
        is_dir: bool,
    ) -> Result<bool, WorkflowEngineError> {
        self.check_ignore_scope(
            &root.chatspeed_scopes,
            path,
            Some(is_dir),
            CHATSPEED_IGNORE_FILE,
        )
    }

    fn check_ignore_rules(
        &self,
        root: &AuthorizedRoot,
        path: &Path,
    ) -> Result<(), WorkflowEngineError> {
        let allowed_by_chatspeed = self.check_chatspeed_ignore(root, path, path.is_dir())?;
        if !allowed_by_chatspeed {
            self.check_gitignore(root, path)?;
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
    fn test_path_guard_csignore_allows_gitignored_file() {
        let root = tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        fs::create_dir(root_path.join("dev_data")).unwrap();
        fs::write(root_path.join(".gitignore"), "dev_data/\n").unwrap();
        fs::write(
            root_path.join(CHATSPEED_IGNORE_FILE),
            "# ChatSpeed can access dev data\n!dev_data/\n!dev_data/**\n",
        )
        .unwrap();
        fs::write(root_path.join("dev_data/fixture.txt"), "fixture").unwrap();
        let guard = PathGuard::new(vec![root_path.clone()], vec![], vec![]);

        assert!(guard
            .validate(&root_path.join("dev_data/fixture.txt"), false, false, false)
            .is_ok());
    }

    #[test]
    fn test_path_guard_csignore_denies_path() {
        let root = tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        fs::write(root_path.join(CHATSPEED_IGNORE_FILE), "private/\n").unwrap();
        fs::create_dir(root_path.join("private")).unwrap();
        fs::write(root_path.join("private/secret.txt"), "secret").unwrap();
        let guard = PathGuard::new(vec![root_path.clone()], vec![], vec![]);

        assert!(guard
            .validate(&root_path.join("private/secret.txt"), false, false, false)
            .is_err());
    }

    #[test]
    fn ignore_scope_collection_prunes_dependency_and_build_directories() {
        let root = tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        for directory in ["target", "node_modules", "__pycache__", ".venv"] {
            let nested = root_path.join(directory).join("nested");
            fs::create_dir_all(&nested).unwrap();
            fs::write(nested.join(CHATSPEED_IGNORE_FILE), "private/\n").unwrap();
        }
        fs::create_dir(root_path.join("src")).unwrap();
        fs::write(
            root_path.join("src").join(CHATSPEED_IGNORE_FILE),
            "private/\n",
        )
        .unwrap();

        let scopes = PathGuard::collect_ignore_scopes(&root_path, CHATSPEED_IGNORE_FILE);

        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].base_dir, root_path.join("src"));
    }

    #[test]
    fn test_path_guard_listing_uses_csignore_not_gitignore() {
        let root = tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        fs::create_dir(root_path.join("dev_data")).unwrap();
        fs::write(root_path.join(".gitignore"), "dev_data/\n").unwrap();
        fs::write(
            root_path.join(CHATSPEED_IGNORE_FILE),
            "!dev_data/\n!dev_data/**\n",
        )
        .unwrap();
        fs::write(root_path.join("dev_data/fixture.txt"), "fixture").unwrap();
        let guard = PathGuard::new(vec![root_path.clone()], vec![], vec![]);

        assert!(guard
            .validate_for_listing(&root_path.join("dev_data"), false)
            .is_ok());
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
    fn test_path_guard_nested_gitignore_does_not_hide_project_root() {
        let root = tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let project_path = root_path.join("downloadClient");
        let src_path = project_path.join("src");
        fs::create_dir(&project_path).unwrap();
        fs::create_dir(&src_path).unwrap();
        fs::write(project_path.join(".gitignore"), "bin\nobj\n").unwrap();

        let guard = PathGuard::new(vec![root_path.clone()], vec![], vec![]);

        assert!(guard.validate(&project_path, false, false, false).is_ok());
        assert!(guard.validate(&src_path, false, false, false).is_ok());
        assert!(guard
            .validate(&project_path.join("bin"), false, false, false)
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

    #[test]
    fn test_path_guard_maps_tmp_alias_into_process_temp_dir() {
        let guard = PathGuard::new(vec![], vec![std::env::temp_dir()], vec![]);
        assert_eq!(
            guard
                .validate(
                    Path::new("/tmp/chatspeed-security-test.txt"),
                    false,
                    true,
                    false,
                )
                .unwrap(),
            std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join("chatspeed-security-test.txt")
        );
    }

    #[test]
    fn test_path_guard_treats_tmp_alias_as_authorized_root() {
        let guard = PathGuard::new(vec![], vec![std::env::temp_dir()], vec![]);
        assert!(guard.is_authorized_root(Path::new("/tmp")));
    }
}
