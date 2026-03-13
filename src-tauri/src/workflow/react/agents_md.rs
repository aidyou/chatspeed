//! AGENTS.md file scanner for ChatSpeed workflow engine.
//!
//! Scans for AGENTS.md files in standard locations:
//! - Global: ~/.chatspeed/AGENTS.md
//! - Project: {project_root}/AGENTS.md

use regex::Regex;
use std::path::{Path, PathBuf};

/// Scope of AGENTS.md file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AgentsScope {
    Global,
    Project,
}

const MAX_INCLUDE_SIZE: u64 = 32 * 1024; // 32KB

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
            .and_then(|p| {
                let content = std::fs::read_to_string(&p).ok()?;
                let parent = p.parent()?;
                Some(Self::process_mentions(&content, parent))
            })
    }

    /// Reads project AGENTS.md from {project_root}/AGENTS.md
    fn read_project(project_root: PathBuf) -> Option<String> {
        let path = project_root.join("AGENTS.md");
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Some(parent) = path.parent() {
                    return Some(Self::process_mentions(&content, parent));
                }
            }
        }
        None
    }

    /// Processes @file mentions in the content.
    /// Replaces mentions with the content of the referenced file if it exists in the same directory.
    fn process_mentions(content: &str, dir: &Path) -> String {
        // Safe regex initialization inside the function
        let re = match Regex::new(r"(?i)@([a-zA-Z0-9_\-\.]+\.md)") {
            Ok(r) => r,
            Err(e) => {
                log::error!("Failed to compile AGENTS.md mention regex: {}", e);
                return content.to_string();
            }
        };

        let mut result = content.to_string();

        // Collect all unique matches to avoid index shifting and redundant disk IO
        let mut replacements = Vec::new();

        for cap in re.captures_iter(content) {
            if let (Some(full_match), Some(file_name)) = (cap.get(0), cap.get(1)) {
                let full_token = full_match.as_str();
                let name_str = file_name.as_str();
                let file_path = dir.join(name_str);

                if file_path.exists() && file_path.is_file() {
                    if let Ok(metadata) = std::fs::metadata(&file_path) {
                        if metadata.len() <= MAX_INCLUDE_SIZE {
                            if let Ok(included_content) = std::fs::read_to_string(&file_path) {
                                replacements.push((full_token.to_string(), included_content));
                            }
                        }
                    }
                }
            }
        }

        // Apply replacements (non-recursively)
        for (token, replacement) in replacements {
            result = result.replace(&token, &replacement);
        }

        result
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
