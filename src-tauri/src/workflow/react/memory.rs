//! Memory management module for ChatSpeed workflow engine.
//!
//! Handles reading and writing of global and project-level memory files.
//! Memory files are stored in markdown format and contain user preferences,
//! constraints, facts, and conventions that should be remembered across sessions.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::error::WorkflowEngineError;

/// Scope of memory - either global or project-specific.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryScope {
    Global,
    Project,
}

/// Memory manager for reading and writing memory files.
///
/// Global memory: `~/.chatspeed/memory.md`
/// Project memory: `~/.chatspeed/project/{transformed_path}/memory.md`
///
/// The project path is transformed to create a unique directory name:
/// - Remove leading slash from absolute path
/// - Replace all slashes with dashes
/// - Example: `/home/user/projects/myapp` → `home-user-projects-myapp`
/// - Final path: `~/.chatspeed/project/home-user-projects-myapp/memory.md`
///
/// This approach keeps project memory out of the project directory itself,
/// avoiding git tracking issues while maintaining accessibility for AI tools.
pub struct MemoryManager {
    /// Path to global memory file.
    global_path: PathBuf,
    /// Path to project memory file (None if no project root).
    project_path: Option<PathBuf>,
    /// Physical project root for project-scoped memory.
    project_root: Option<PathBuf>,
    /// Stable project key used by candidate persistence.
    project_key: Option<String>,
}

impl MemoryManager {
    /// Creates a new MemoryManager.
    ///
    /// # Arguments
    /// * `project_root` - Optional project root directory. If None, project-level
    ///   memory operations will be silently skipped.
    pub fn new(project_root: Option<PathBuf>) -> Self {
        let global_path = dirs::home_dir()
            .map(|h| h.join(".chatspeed").join("memory.md"))
            .unwrap_or_else(|| PathBuf::from(".chatspeed/memory.md"));

        let project_path = project_root.as_ref().and_then(|p| {
            let chatspeed_dir = dirs::home_dir()?.join(".chatspeed");
            let transformed = Self::project_key_from_root(p);
            Some(
                chatspeed_dir
                    .join("project")
                    .join(&transformed)
                    .join("memory.md"),
            )
        });
        let project_key = project_root
            .as_ref()
            .map(|root| Self::project_key_from_root(root));

        Self {
            global_path,
            project_path,
            project_root,
            project_key,
        }
    }

    pub fn project_key_from_root(root: &Path) -> String {
        let path_str = root.to_string_lossy();
        path_str
            .strip_prefix('/')
            .unwrap_or(&path_str)
            .replace('/', "-")
    }

    /// Reads memory content from the specified scope.
    ///
    /// # Arguments
    /// * `scope` - MemoryScope::Global or MemoryScope::Project
    ///
    /// # Returns
    /// * `Ok(Some(content))` - If memory file exists and is readable
    /// * `Ok(None)` - If memory file does not exist
    /// * `Err(WorkflowEngineError)` - If file cannot be read
    pub fn read(&self, scope: MemoryScope) -> Result<Option<String>, WorkflowEngineError> {
        let path = match scope {
            MemoryScope::Global => &self.global_path,
            MemoryScope::Project => match &self.project_path {
                Some(p) => p,
                None => return Ok(None), // No project path, skip silently
            },
        };

        if !path.exists() {
            return Ok(None);
        }

        std::fs::read_to_string(path)
            .map(Some)
            .map_err(|e| WorkflowEngineError::General(format!("Failed to read memory file: {}", e)))
    }

    /// Reads the last N non-empty lines of memory content from the specified scope.
    /// Each line is trimmed and empty lines are removed to maximize context efficiency.
    pub fn read_last_n_lines(
        &self,
        scope: MemoryScope,
        n: usize,
    ) -> Result<Option<String>, WorkflowEngineError> {
        let content = self.read(scope)?;
        match content {
            Some(s) => {
                let lines: Vec<String> = s
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();

                if lines.is_empty() {
                    return Ok(None);
                }

                if lines.len() <= n {
                    Ok(Some(lines.join("\n")))
                } else {
                    let start = lines.len() - n;
                    Ok(Some(lines[start..].join("\n")))
                }
            }
            None => Ok(None),
        }
    }

    /// Writes memory content to the specified scope (overwrites existing content).
    ///
    /// # Arguments
    /// * `scope` - MemoryScope::Global or MemoryScope::Project
    /// * `content` - Memory content in markdown format
    ///
    /// # Returns
    /// * `Ok(())` - If memory was written successfully
    /// * `Err(WorkflowEngineError)` - If file cannot be written
    pub fn write(&self, scope: MemoryScope, content: &str) -> Result<(), WorkflowEngineError> {
        let path = match scope {
            MemoryScope::Global => &self.global_path,
            MemoryScope::Project => match &self.project_path {
                Some(p) => p,
                None => return Ok(()), // No project path, skip silently
            },
        };

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                WorkflowEngineError::General(format!("Failed to create directory: {}", e))
            })?;
        }

        std::fs::write(path, content).map_err(|e| {
            WorkflowEngineError::General(format!("Failed to write memory file: {}", e))
        })
    }

    /// Returns true if project memory is available (i.e., project_root was provided).
    pub fn has_project_memory(&self) -> bool {
        self.project_path.is_some()
    }

    pub fn project_key(&self) -> Option<&str> {
        self.project_key.as_deref()
    }

    pub fn project_root(&self) -> Option<&Path> {
        self.project_root.as_deref()
    }

    pub fn merge_entries(
        &self,
        scope: MemoryScope,
        entries_by_category: &BTreeMap<String, Vec<String>>,
    ) -> Result<Option<String>, WorkflowEngineError> {
        if entries_by_category.is_empty() {
            return Ok(None);
        }

        let current = self.read(scope)?.unwrap_or_default();
        let mut sections = parse_memory_sections(&current);
        let before = render_memory_sections(&sections);

        for (category, entries) in entries_by_category {
            let bucket = sections.entry(category.clone()).or_default();
            for entry in entries {
                if !bucket.iter().any(|existing| {
                    normalize_memory_entry(existing) == normalize_memory_entry(entry)
                }) {
                    bucket.push(entry.clone());
                }
            }
        }

        let after = render_memory_sections(&sections);
        if normalize_memory_entry(&before) == normalize_memory_entry(&after) {
            return Ok(None);
        }

        self.write(scope, &after)?;
        Ok(Some(after))
    }

    /// Gets the global memory file path (for debugging purposes).
    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    pub fn global_path(&self) -> &PathBuf {
        &self.global_path
    }

    /// Gets the project memory file path (for debugging purposes).
    #[cfg(debug_assertions)]
    #[allow(dead_code)]
    pub fn project_path(&self) -> Option<&PathBuf> {
        self.project_path.as_ref()
    }
}

/// Result of memory analysis from the AI.
///
/// The AI returns this structure when analyzing user inputs for new memories
/// to record. Values are None if no changes are needed.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryAnalysisResult {
    /// Updated global memory content. Null if no changes.
    #[serde(default)]
    pub global_memory: Option<String>,

    /// Updated project memory content. Null if no changes.
    #[serde(default)]
    pub project_memory: Option<String>,

    /// Extracted global candidates from the current session.
    #[serde(default)]
    pub global_candidates: Vec<MemoryCandidateDraft>,

    /// Extracted project candidates from the current session.
    #[serde(default)]
    pub project_candidates: Vec<MemoryCandidateDraft>,

    /// Reasoning for the changes made.
    #[serde(default)]
    #[allow(dead_code)]
    pub reasoning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCandidateDraft {
    pub category: String,
    pub content: String,
    pub confidence: f32,
    pub explicitness: i32,
}

pub fn normalize_memory_entry(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("- ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub fn parse_memory_sections(content: &str) -> BTreeMap<String, Vec<String>> {
    let mut sections: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut current_category: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(category) = trimmed.strip_prefix("## ") {
            current_category = Some(category.trim().to_string());
            sections
                .entry(category.trim().to_string())
                .or_insert_with(Vec::new);
            continue;
        }

        if let Some(entry) = trimmed.strip_prefix("- ") {
            if let Some(category) = current_category.as_ref() {
                let bucket = sections.entry(category.clone()).or_default();
                let entry = entry.trim().to_string();
                if !entry.is_empty()
                    && !bucket.iter().any(|existing| {
                        normalize_memory_entry(existing) == normalize_memory_entry(&entry)
                    })
                {
                    bucket.push(entry);
                }
            }
        }
    }

    sections
}

pub fn render_memory_sections(sections: &BTreeMap<String, Vec<String>>) -> String {
    let category_order = [
        "preference",
        "constraint",
        "fact",
        "skill",
        "convention",
        "architecture",
        "tooling",
        "config",
    ];

    let mut ordered_categories: Vec<String> = sections.keys().cloned().collect();
    ordered_categories.sort_by_key(|category| {
        category_order
            .iter()
            .position(|item| item == category)
            .unwrap_or(category_order.len())
    });

    let mut blocks = Vec::new();
    for category in ordered_categories {
        let Some(entries) = sections.get(&category) else {
            continue;
        };
        if entries.is_empty() {
            continue;
        }

        let mut deduped = entries.clone();
        deduped.sort();
        deduped.dedup_by(|a, b| normalize_memory_entry(a) == normalize_memory_entry(b));

        let mut lines = vec![format!("## {}", category)];
        for entry in deduped {
            lines.push(format!("- {}", entry));
        }
        blocks.push(lines.join("\n"));
    }

    blocks.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_manager_creation() {
        let manager = MemoryManager::new(None);
        // Should have global path set
        let global = manager.read(MemoryScope::Global).unwrap();
        assert!(global.is_none()); // File doesn't exist yet
    }

    #[test]
    fn test_memory_manager_with_project() {
        let temp_dir = std::env::temp_dir();
        let manager = MemoryManager::new(Some(temp_dir.clone()));

        // Should have both global and project paths
        let global = manager.read(MemoryScope::Global).unwrap();
        assert!(global.is_none());

        let project = manager.read(MemoryScope::Project).unwrap();
        assert!(project.is_none());
    }

    #[test]
    fn test_memory_scope_serialization() {
        let global = MemoryScope::Global;
        let project = MemoryScope::Project;

        // Should serialize to lowercase
        assert!(serde_json::to_string(&global)
            .unwrap()
            .contains("\"global\""));
        assert!(serde_json::to_string(&project)
            .unwrap()
            .contains("\"project\""));
    }

    #[test]
    fn test_project_key_from_root() {
        let key = MemoryManager::project_key_from_root(Path::new("/Users/test/project/demo"));
        assert_eq!(key, "Users-test-project-demo");
    }

    #[test]
    fn test_parse_and_render_memory_sections_deduplicates_entries() {
        let content = "## preference\n- Use Rust\n- Use Rust\n## constraint\n- Avoid unwrap()";
        let sections = parse_memory_sections(content);
        let rendered = render_memory_sections(&sections);
        assert_eq!(sections["preference"].len(), 1);
        assert!(rendered.contains("## preference"));
        assert!(rendered.contains("- Use Rust"));
    }
}
