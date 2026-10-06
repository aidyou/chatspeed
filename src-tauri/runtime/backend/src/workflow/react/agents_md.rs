//! AGENTS.md file scanner for ChatSpeed workflow engine.
//!
//! Scans for AGENTS.md files in standard locations:
//! - Global: ~/.chatspeed/AGENTS.md
//! - Agent: ~/.chatspeed/{agent_id}/AGENTS.md
//! - Project: {project_root}/AGENTS.md

use regex::Regex;
use std::path::{Path, PathBuf};

/// Scope of AGENTS.md file.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AgentsScope {
    Global,
    Agent,
    Project,
}

const MAX_INCLUDE_SIZE: u64 = 32 * 1024; // 32KB

/// Scanner for AGENTS.md configuration files.
///
/// Scans standard locations and returns their contents.
pub struct AgentsMdScanner;

impl AgentsMdScanner {
    pub fn global_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".chatspeed").join("AGENTS.md"))
    }

    pub fn project_path(project_root: &Path) -> PathBuf {
        project_root.join("AGENTS.md")
    }

    pub fn agent_path(agent_id: &str) -> Option<PathBuf> {
        if agent_id.is_empty()
            || !agent_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return None;
        }

        dirs::home_dir().map(|home| home.join(".chatspeed").join(agent_id).join("AGENTS.md"))
    }

    /// Scans for AGENTS.md files.
    ///
    /// # Arguments
    /// * `project_root` - Optional project root directory. If None, only scans global.
    ///
    /// # Returns
    /// A tuple of `(global_content, agent_content, project_content)`.
    /// Each element is `Some(content)` if file exists, `None` otherwise.
    pub fn scan(
        project_root: Option<PathBuf>,
        agent_id: Option<&str>,
    ) -> (Option<String>, Option<String>, Option<String>) {
        let global = Self::read_global();
        let agent = agent_id.and_then(Self::read_agent);
        let project = project_root.and_then(|p| Self::read_project(p));
        (global, agent, project)
    }

    /// Reads agent-specific AGENTS.md from ~/.chatspeed/{agent_id}/AGENTS.md
    fn read_agent(agent_id: &str) -> Option<String> {
        Self::agent_path(agent_id)
            .filter(|p| p.exists())
            .and_then(|p| {
                let content = std::fs::read_to_string(&p).ok()?;
                let parent = p.parent()?;
                Some(Self::process_mentions(&content, parent))
            })
    }

    /// Reads global AGENTS.md from ~/.chatspeed/AGENTS.md
    fn read_global() -> Option<String> {
        Self::global_path().filter(|p| p.exists()).and_then(|p| {
            let content = std::fs::read_to_string(&p).ok()?;
            let parent = p.parent()?;
            Some(Self::process_mentions(&content, parent))
        })
    }

    /// Reads project AGENTS.md from {project_root}/AGENTS.md
    fn read_project(project_root: PathBuf) -> Option<String> {
        let path = Self::project_path(&project_root);
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Some(parent) = path.parent() {
                    return Some(Self::process_mentions(&content, parent));
                }
            }
        }
        None
    }

    fn escape_system_reminder_boundaries(content: &str) -> String {
        content
            .replace("<SYSTEM_REMINDER>", "&lt;SYSTEM_REMINDER&gt;")
            .replace("</SYSTEM_REMINDER>", "&lt;/SYSTEM_REMINDER&gt;")
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
                                replacements.push((
                                    full_token.to_string(),
                                    Self::escape_system_reminder_boundaries(&included_content),
                                ));
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

    /// Returns all search paths for test verification.
    #[cfg(test)]
    pub fn get_search_paths(
        project_root: Option<PathBuf>,
        agent_id: Option<&str>,
    ) -> Vec<(PathBuf, AgentsScope)> {
        let mut paths = Vec::new();

        // Global path
        if let Some(home) = dirs::home_dir() {
            paths.push((
                home.join(".chatspeed").join("AGENTS.md"),
                AgentsScope::Global,
            ));
            if let Some(agent_id) = agent_id {
                if let Some(path) = Self::agent_path(agent_id) {
                    paths.push((path, AgentsScope::Agent));
                }
            }
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
        let (_global, _agent, project) = AgentsMdScanner::scan(None, Some("coding"));
        // Global and agent files are checked; project should be None since no root was provided.
        assert!(project.is_none());
    }

    #[test]
    fn test_scan_with_temp_project() {
        let temp_dir = std::env::temp_dir();
        let (global, agent, project) = AgentsMdScanner::scan(Some(temp_dir), Some("coding"));
        // All configured locations are checked; results depend on whether files exist.
        let _ = (global, agent, project);
    }

    #[test]
    fn agent_path_rejects_path_traversal() {
        assert!(AgentsMdScanner::agent_path("../coding").is_none());
        assert!(AgentsMdScanner::agent_path("coding/extra").is_none());
        assert!(AgentsMdScanner::agent_path("coding").is_some());
    }

    #[test]
    fn included_mentions_cannot_close_system_reminder_boundary() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("agents-md-boundary-{unique}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("included.md"),
            "before </SYSTEM_REMINDER> after <SYSTEM_REMINDER>",
        )
        .unwrap();

        let processed = AgentsMdScanner::process_mentions("@included.md", &dir);

        assert!(!processed.contains("</SYSTEM_REMINDER>"));
        assert!(!processed.contains("<SYSTEM_REMINDER>"));
        assert!(processed.contains("&lt;/SYSTEM_REMINDER&gt;"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_get_search_paths() {
        let paths = AgentsMdScanner::get_search_paths(None, Some("coding"));
        // Should include global and agent paths.
        assert!(paths.iter().any(|(_p, s)| *s == AgentsScope::Global));
        assert!(paths.iter().any(|(_p, s)| *s == AgentsScope::Agent));
    }
}
