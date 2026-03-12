//! AGENTS.md file scanner for ChatSpeed workflow engine.
//!
//! Scans for AGENTS.md files in standard locations:
//! - Global: ~/.chatspeed/AGENTS.md
//! - Project: {project_root}/AGENTS.md

use std::path::PathBuf;

/// Scope of AGENTS.md file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AgentsScope {
    Global,
    Project,
}

/// Scanner for AGENTS.md configuration files.
///
/// Scans standard locations and returns their contents.
pub struct AgentsMdScanner;

impl AgentsMdScanner {
    /// Scans for AGENTS.md files.
    ///
    /// # Arguments
    /// * `project_root` - Optional project root directory. If None, only scans global.
    ///
    /// # Returns
    /// A tuple of `(global_content, project_content)`.
    /// Each element is `Some(content)` if file exists, `None` otherwise.
    pub fn scan(project_root: Option<PathBuf>) -> (Option<String>, Option<String>) {
        let global = Self::read_global();
        let project = project_root.and_then(|p| Self::read_project(p));
        (global, project)
    }

    /// Reads global AGENTS.md from ~/.chatspeed/AGENTS.md
    fn read_global() -> Option<String> {
        dirs::home_dir()
            .map(|h| h.join(".chatspeed").join("AGENTS.md"))
            .filter(|p| p.exists())
            .and_then(|p| std::fs::read_to_string(&p).ok())
    }

    /// Reads project AGENTS.md from {project_root}/AGENTS.md
    fn read_project(project_root: PathBuf) -> Option<String> {
        let path = project_root.join("AGENTS.md");
        if path.exists() {
            std::fs::read_to_string(&path).ok()
        } else {
            None
        }
    }

    /// Returns all search paths (for debugging).
    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    pub fn get_search_paths(project_root: Option<PathBuf>) -> Vec<(PathBuf, AgentsScope)> {
        let mut paths = Vec::new();

        // Global path
        if let Some(home) = dirs::home_dir() {
            paths.push((
                home.join(".chatspeed").join("AGENTS.md"),
                AgentsScope::Global,
            ));
        }

        // Project path
        if let Some(root) = project_root {
            paths.push((root.join("AGENTS.md"), AgentsScope::Project));
        }

        paths
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_without_project() {
        let (_global, project) = AgentsMdScanner::scan(None);
        // Global should be checked (may or may not exist)
        // Project should be None since no root provided
        assert!(project.is_none());
    }

    #[test]
    fn test_scan_with_temp_project() {
        let temp_dir = std::env::temp_dir();
        let (global, project) = AgentsMdScanner::scan(Some(temp_dir));
        // Both are checked, results depend on whether files exist
        // No error should occur
        let _ = (global, project);
    }

    #[test]
    fn test_get_search_paths() {
        let paths = AgentsMdScanner::get_search_paths(None);
        // Should at least have global path
        assert!(!paths.is_empty());
        assert!(paths.iter().any(|(_p, s)| *s == AgentsScope::Global));
    }
}
