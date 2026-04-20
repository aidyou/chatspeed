//! Child task management for Phase 7 - Single-layer Call model
//!
//! This module provides functionality for parent tasks to wait on child tasks
//! and for child tasks to notify parents upon completion.

use dashmap::DashMap;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

/// Global registry for tracking parent-child task relationships
pub struct ChildTaskRegistry {
    /// Maps child task ID to parent session info
    parent_mapping: DashMap<String, ParentSessionInfo>,
    /// Reverse index of parent session ID to its child task IDs
    child_mapping: DashMap<String, HashSet<String>>,
}

#[derive(Debug, Clone)]
pub struct ParentSessionInfo {
    pub parent_session_id: String,
}

impl ChildTaskRegistry {
    pub fn new() -> Self {
        Self {
            parent_mapping: DashMap::new(),
            child_mapping: DashMap::new(),
        }
    }

    /// Register a child task with its parent
    pub fn register_child_task(&self, child_task_id: String, parent_session_id: String) {
        let child_task_id_for_reverse = child_task_id.clone();
        self.parent_mapping.insert(
            child_task_id.clone(),
            ParentSessionInfo {
                parent_session_id: parent_session_id.clone(),
            },
        );
        let mut entry = self.child_mapping.entry(parent_session_id).or_default();
        entry.insert(child_task_id_for_reverse);
    }

    /// Unregister a child task (when it completes or fails)
    pub fn unregister_child_task(&self, child_task_id: &str) -> Option<ParentSessionInfo> {
        self.parent_mapping.remove(child_task_id).map(|(_, v)| {
            if let Some(mut children) = self.child_mapping.get_mut(&v.parent_session_id) {
                children.remove(child_task_id);
                if children.is_empty() {
                    drop(children);
                    self.child_mapping.remove(&v.parent_session_id);
                }
            }
            v
        })
    }

    /// Get parent session info for a child task
    #[cfg(test)]
    pub fn get_parent_info(&self, child_task_id: &str) -> Option<ParentSessionInfo> {
        self.parent_mapping.get(child_task_id).map(|v| v.clone())
    }

    /// Check if a task is a child task
    #[cfg(test)]
    pub fn is_child_task(&self, task_id: &str) -> bool {
        self.parent_mapping.contains_key(task_id)
    }

    /// List all child task IDs currently owned by the given parent session.
    pub fn list_child_tasks_for_parent(&self, parent_session_id: &str) -> Vec<String> {
        self.child_mapping
            .get(parent_session_id)
            .map(|children| children.iter().cloned().collect())
            .unwrap_or_default()
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

#[derive(Debug, Clone, PartialEq)]
pub struct ChildTaskResolution {
    pub child_task_id: String,
    pub status: String,
    pub content: String,
    pub is_error: bool,
}

pub fn resolve_child_task_completion(
    expected_child_task_id: &mut Option<String>,
    child_sessions: &mut Vec<String>,
    incoming_child_task_id: &str,
    result: &Value,
) -> Option<ChildTaskResolution> {
    if expected_child_task_id.as_deref() != Some(incoming_child_task_id) {
        return None;
    }

    let status = result
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("completed")
        .to_string();
    let content = result
        .get("summary")
        .and_then(|s| s.as_str())
        .or_else(|| result.get("error").and_then(|e| e.as_str()))
        .unwrap_or("Child task completed")
        .to_string();
    let is_error = matches!(status.as_str(), "failed" | "cancelled" | "interrupted");

    child_sessions.retain(|id| id != incoming_child_task_id);
    *expected_child_task_id = None;

    Some(ChildTaskResolution {
        child_task_id: incoming_child_task_id.to_string(),
        status,
        content,
        is_error,
    })
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

        let removed = registry.unregister_child_task("child_1").unwrap();
        assert_eq!(removed.parent_session_id, "parent_1");
        assert!(!registry.is_child_task("child_1"));
        assert!(registry.list_child_tasks_for_parent("parent_1").is_empty());
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

        let parent_1_children = registry.list_child_tasks_for_parent("parent_1");

        assert_eq!(parent_1_children.len(), 2);
        assert!(parent_1_children.contains(&"child_1".to_string()));
        assert!(parent_1_children.contains(&"child_2".to_string()));
    }

    #[test]
    fn test_resolve_child_task_completion_completed() {
        let mut waiting_on = Some("child_1".to_string());
        let mut child_sessions = vec!["child_1".to_string(), "child_2".to_string()];

        let resolution = resolve_child_task_completion(
            &mut waiting_on,
            &mut child_sessions,
            "child_1",
            &serde_json::json!({
                "status": "completed",
                "summary": "done"
            }),
        )
        .unwrap();

        assert_eq!(resolution.status, "completed");
        assert_eq!(resolution.content, "done");
        assert!(!resolution.is_error);
        assert_eq!(waiting_on, None);
        assert_eq!(child_sessions, vec!["child_2".to_string()]);
    }

    #[test]
    fn test_resolve_child_task_completion_failed() {
        let mut waiting_on = Some("child_1".to_string());
        let mut child_sessions = vec!["child_1".to_string()];

        let resolution = resolve_child_task_completion(
            &mut waiting_on,
            &mut child_sessions,
            "child_1",
            &serde_json::json!({
                "status": "failed",
                "error": "boom"
            }),
        )
        .unwrap();

        assert_eq!(resolution.status, "failed");
        assert_eq!(resolution.content, "boom");
        assert!(resolution.is_error);
        assert_eq!(waiting_on, None);
        assert!(child_sessions.is_empty());
    }

    #[test]
    fn test_resolve_child_task_completion_cancelled() {
        let mut waiting_on = Some("child_1".to_string());
        let mut child_sessions = vec!["child_1".to_string()];

        let resolution = resolve_child_task_completion(
            &mut waiting_on,
            &mut child_sessions,
            "child_1",
            &serde_json::json!({
                "status": "cancelled",
                "error": "user cancelled"
            }),
        )
        .unwrap();

        assert_eq!(resolution.status, "cancelled");
        assert_eq!(resolution.content, "user cancelled");
        assert!(resolution.is_error);
        assert_eq!(waiting_on, None);
        assert!(child_sessions.is_empty());
    }

    #[test]
    fn test_resolve_child_task_completion_interrupted() {
        let mut waiting_on = Some("child_1".to_string());
        let mut child_sessions = vec!["child_1".to_string()];

        let resolution = resolve_child_task_completion(
            &mut waiting_on,
            &mut child_sessions,
            "child_1",
            &serde_json::json!({
                "status": "interrupted",
                "error": "process restarted"
            }),
        )
        .unwrap();

        assert_eq!(resolution.status, "interrupted");
        assert_eq!(resolution.content, "process restarted");
        assert!(resolution.is_error);
        assert_eq!(waiting_on, None);
        assert!(child_sessions.is_empty());
    }

    #[test]
    fn test_resolve_child_task_completion_ignores_mismatch() {
        let mut waiting_on = Some("child_1".to_string());
        let mut child_sessions = vec!["child_1".to_string(), "child_2".to_string()];

        let resolution = resolve_child_task_completion(
            &mut waiting_on,
            &mut child_sessions,
            "child_2",
            &serde_json::json!({
                "status": "completed",
                "summary": "done"
            }),
        );

        assert!(resolution.is_none());
        assert_eq!(waiting_on, Some("child_1".to_string()));
        assert_eq!(
            child_sessions,
            vec!["child_1".to_string(), "child_2".to_string()]
        );
    }
}
