//! Memory management module for ChatSpeed workflow engine.
//!
//! Handles reading and writing of global and project-level memory files.
//! Memory files are stored in markdown format and contain user preferences,
//! constraints, facts, and conventions that should be remembered across sessions.

use std::path::PathBuf;
use serde::{Deserialize, Serialize};

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
/// Project memory: `{project_root}/.cs/memory.md`
pub struct MemoryManager {
    /// Path to global memory file.
    global_path: PathBuf,
    /// Path to project memory file (None if no project root).
    project_path: Option<PathBuf>,
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

        let project_path = project_root.map(|p| p.join(".cs").join("memory.md"));

        Self {
            global_path,
            project_path,
        }
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
            .map_err(|e| {
                WorkflowEngineError::General(format!("Failed to read memory file: {}", e))
            })
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

        std::fs::write(path, content)
            .map_err(|e| {
                WorkflowEngineError::General(format!("Failed to write memory file: {}", e))
            })
    }

    /// Returns true if project memory is available (i.e., project_root was provided).
    pub fn has_project_memory(&self) -> bool {
        self.project_path.is_some()
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

    /// Reasoning for the changes made.
    #[serde(default)]
    #[allow(dead_code)]
    pub reasoning: Option<String>,
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
        assert!(serde_json::to_string(&global).unwrap().contains("\"global\""));
        assert!(serde_json::to_string(&project).unwrap().contains("\"project\""));
    }
}