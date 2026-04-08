//! Child task management for Phase 7 - Single-layer Call model
//!
//! This module provides functionality for parent tasks to wait on child tasks
//! and for child tasks to notify parents upon completion.

use dashmap::DashMap;
use serde_json::Value;
use std::sync::Arc;

/// Global registry for tracking parent-child task relationships
pub struct ChildTaskRegistry {
    /// Maps child task ID to parent session info
    parent_mapping: DashMap<String, ParentSessionInfo>,
}

#[derive(Debug, Clone)]
pub struct ParentSessionInfo {
    pub parent_session_id: String,
    pub child_task_id: String,
}

impl ChildTaskRegistry {
    pub fn new() -> Self {
        Self {
            parent_mapping: DashMap::new(),
        }
    }

    /// Register a child task with its parent
    pub fn register_child_task(&self, child_task_id: String, parent_session_id: String) {
        self.parent_mapping.insert(
            child_task_id.clone(),
            ParentSessionInfo {
                parent_session_id,
                child_task_id,
            },
        );
    }

    /// Unregister a child task (when it completes or fails)
    pub fn unregister_child_task(&self, child_task_id: &str) -> Option<ParentSessionInfo> {
        self.parent_mapping.remove(child_task_id).map(|(_, v)| v)
    }

    /// Get parent session info for a child task
    pub fn get_parent_info(&self, child_task_id: &str) -> Option<ParentSessionInfo> {
        self.parent_mapping.get(child_task_id).map(|v| v.clone())
    }

    /// Check if a task is a child task
    pub fn is_child_task(&self, task_id: &str) -> bool {
        self.parent_mapping.contains_key(task_id)
    }
}

impl Default for ChildTaskRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Lazy-initialized global registry
pub fn get_child_task_registry() -> Arc<ChildTaskRegistry> {
    use std::sync::OnceLock;
    static REGISTRY: OnceLock<Arc<ChildTaskRegistry>> = OnceLock::new();
    REGISTRY
        .get_or_init(|| Arc::new(ChildTaskRegistry::new()))
        .clone()
}

/// Result returned when a child task is spawned in Call mode
#[derive(Debug, Clone)]
pub enum TaskCallResult {
    /// Task completed synchronously with result
    Completed { result: Value },
    /// Task spawned and parent should wait
    Waiting {
        child_task_id: String,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_child_task_registry_register_and_unregister() {
        let registry = ChildTaskRegistry::new();

        registry.register_child_task("child_1".to_string(), "parent_1".to_string());

        assert!(registry.is_child_task("child_1"));
        assert!(!registry.is_child_task("child_2"));

        let parent_info = registry.get_parent_info("child_1").unwrap();
        assert_eq!(parent_info.parent_session_id, "parent_1");
        assert_eq!(parent_info.child_task_id, "child_1");

        let removed = registry.unregister_child_task("child_1").unwrap();
        assert_eq!(removed.parent_session_id, "parent_1");
        assert!(!registry.is_child_task("child_1"));
    }

    #[test]
    fn test_child_task_registry_multiple_children() {
        let registry = ChildTaskRegistry::new();

        registry.register_child_task("child_1".to_string(), "parent_1".to_string());
        registry.register_child_task("child_2".to_string(), "parent_1".to_string());
        registry.register_child_task("child_3".to_string(), "parent_2".to_string());

        assert!(registry.is_child_task("child_1"));
        assert!(registry.is_child_task("child_2"));
        assert!(registry.is_child_task("child_3"));

        let parent_1_children: Vec<_> = registry
            .parent_mapping
            .iter()
            .filter(|v| v.parent_session_id == "parent_1")
            .map(|v| v.child_task_id.clone())
            .collect();

        assert_eq!(parent_1_children.len(), 2);
        assert!(parent_1_children.contains(&"child_1".to_string()));
        assert!(parent_1_children.contains(&"child_2".to_string()));
    }
}
