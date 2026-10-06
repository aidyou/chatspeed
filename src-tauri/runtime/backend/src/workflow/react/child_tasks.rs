//! Sub-agent management for Phase 7 - Single-layer Call model
//!
//! This module provides functionality for parent sessions to wait on sub-agents
//! and for sub-agents to notify parents upon completion.

use dashmap::DashMap;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

/// Global registry for tracking parent-sub-agent relationships
pub struct SubAgentRegistry {
    /// Maps sub-agent ID to parent session info
    parent_mapping: DashMap<String, ParentSessionInfo>,
    /// Reverse index of parent session ID to its sub-agent IDs
    child_mapping: DashMap<String, HashSet<String>>,
}

#[derive(Debug, Clone)]
pub struct ParentSessionInfo {
    pub parent_session_id: String,
}

impl SubAgentRegistry {
    pub fn new() -> Self {
        Self {
            parent_mapping: DashMap::new(),
            child_mapping: DashMap::new(),
        }
    }

    /// Register a sub-agent with its parent
    pub fn register_sub_agent(&self, sub_agent_id: String, parent_session_id: String) {
        let sub_agent_id_for_reverse = sub_agent_id.clone();
        self.parent_mapping.insert(
            sub_agent_id.clone(),
            ParentSessionInfo {
                parent_session_id: parent_session_id.clone(),
            },
        );
        let mut entry = self.child_mapping.entry(parent_session_id).or_default();
        entry.insert(sub_agent_id_for_reverse);
    }

    /// Unregister a sub-agent (when it completes or fails)
    pub fn unregister_sub_agent(&self, sub_agent_id: &str) -> Option<ParentSessionInfo> {
        self.parent_mapping.remove(sub_agent_id).map(|(_, v)| {
            if let Some(mut children) = self.child_mapping.get_mut(&v.parent_session_id) {
                children.remove(sub_agent_id);
                if children.is_empty() {
                    drop(children);
                    self.child_mapping.remove(&v.parent_session_id);
                }
            }
            v
        })
    }

    /// Get parent session info for a sub-agent
    #[cfg(test)]
    pub fn get_parent_info(&self, sub_agent_id: &str) -> Option<ParentSessionInfo> {
        self.parent_mapping.get(sub_agent_id).map(|v| v.clone())
    }

    /// Check if a task is a sub-agent
    #[cfg(test)]
    pub fn is_sub_agent(&self, task_id: &str) -> bool {
        self.parent_mapping.contains_key(task_id)
    }

    /// List all sub-agent IDs currently owned by the given parent session.
    pub fn list_sub_agents_for_parent(&self, parent_session_id: &str) -> Vec<String> {
        self.child_mapping
            .get(parent_session_id)
            .map(|children| children.iter().cloned().collect())
            .unwrap_or_default()
    }
}

impl Default for SubAgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Lazy-initialized global registry
pub fn get_sub_agent_registry() -> Arc<SubAgentRegistry> {
    use std::sync::OnceLock;
    static REGISTRY: OnceLock<Arc<SubAgentRegistry>> = OnceLock::new();
    REGISTRY
        .get_or_init(|| Arc::new(SubAgentRegistry::new()))
        .clone()
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubAgentResolution {
    pub sub_agent_id: String,
    pub status: String,
    pub content: String,
    pub summary: Option<String>,
    pub is_error: bool,
}

pub fn resolve_sub_agent_completion(
    expected_sub_agent_id: &mut Option<String>,
    sub_agent_sessions: &mut Vec<String>,
    incoming_sub_agent_id: &str,
    result: &Value,
) -> Option<SubAgentResolution> {
    if expected_sub_agent_id.as_deref() != Some(incoming_sub_agent_id) {
        return None;
    }

    let status = result
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("completed")
        .to_string();
    let content = result
        .get("result")
        .and_then(|s| s.as_str())
        .or_else(|| result.get("summary").and_then(|s| s.as_str()))
        .or_else(|| result.get("error").and_then(|e| e.as_str()))
        .unwrap_or("Sub-agent completed")
        .to_string();
    let summary = result
        .get("summary")
        .and_then(|s| s.as_str())
        .map(str::to_string);
    let is_error = matches!(status.as_str(), "failed" | "cancelled" | "interrupted");

    sub_agent_sessions.retain(|id| id != incoming_sub_agent_id);
    *expected_sub_agent_id = None;

    Some(SubAgentResolution {
        sub_agent_id: incoming_sub_agent_id.to_string(),
        status,
        content,
        summary,
        is_error,
    })
}

pub fn render_call_mode_sub_agent_tool_result(
    sub_agent_id: &str,
    status: &str,
    task: Option<&str>,
    result: &str,
    summary: Option<&str>,
    reused: bool,
) -> String {
    let mut output = format!(
        "<tool_result tool=\"sub_agent_run\" id=\"{}\" mode=\"call\" status=\"{}\"",
        sub_agent_id, status
    );
    if reused {
        output.push_str(" reused=\"true\"");
    }
    output.push_str(">\n");

    if let Some(task) = task.filter(|task| !task.trim().is_empty()) {
        output.push_str("<Task>\n");
        output.push_str(task.trim());
        output.push_str("\n</Task>\n");
    }

    output.push_str("<Result>\n");
    output.push_str(result.trim());
    output.push_str("\n</Result>\n");

    if let Some(summary) = summary.filter(|summary| !summary.trim().is_empty()) {
        output.push_str("<Summary>\n");
        output.push_str(summary.trim());
        output.push_str("\n</Summary>\n");
    }

    output.push_str("</tool_result>");
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sub_agent_registry_register_and_unregister() {
        let registry = SubAgentRegistry::new();

        registry.register_sub_agent("subagent_1".to_string(), "parent_1".to_string());

        assert!(registry.is_sub_agent("subagent_1"));
        assert!(!registry.is_sub_agent("subagent_2"));

        let parent_info = registry.get_parent_info("subagent_1").unwrap();
        assert_eq!(parent_info.parent_session_id, "parent_1");

        let removed = registry.unregister_sub_agent("subagent_1").unwrap();
        assert_eq!(removed.parent_session_id, "parent_1");
        assert!(!registry.is_sub_agent("subagent_1"));
        assert!(registry.list_sub_agents_for_parent("parent_1").is_empty());
    }

    #[test]
    fn test_sub_agent_registry_multiple_children() {
        let registry = SubAgentRegistry::new();

        registry.register_sub_agent("subagent_1".to_string(), "parent_1".to_string());
        registry.register_sub_agent("subagent_2".to_string(), "parent_1".to_string());
        registry.register_sub_agent("subagent_3".to_string(), "parent_2".to_string());

        assert!(registry.is_sub_agent("subagent_1"));
        assert!(registry.is_sub_agent("subagent_2"));
        assert!(registry.is_sub_agent("subagent_3"));

        let parent_1_children = registry.list_sub_agents_for_parent("parent_1");

        assert_eq!(parent_1_children.len(), 2);
        assert!(parent_1_children.contains(&"subagent_1".to_string()));
        assert!(parent_1_children.contains(&"subagent_2".to_string()));
    }

    #[test]
    fn test_resolve_sub_agent_completion_completed() {
        let mut waiting_on = Some("subagent_1".to_string());
        let mut sub_agent_sessions = vec!["subagent_1".to_string(), "subagent_2".to_string()];

        let resolution = resolve_sub_agent_completion(
            &mut waiting_on,
            &mut sub_agent_sessions,
            "subagent_1",
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
        assert_eq!(sub_agent_sessions, vec!["subagent_2".to_string()]);
    }

    #[test]
    fn test_resolve_sub_agent_completion_failed() {
        let mut waiting_on = Some("subagent_1".to_string());
        let mut sub_agent_sessions = vec!["subagent_1".to_string()];

        let resolution = resolve_sub_agent_completion(
            &mut waiting_on,
            &mut sub_agent_sessions,
            "subagent_1",
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
        assert!(sub_agent_sessions.is_empty());
    }

    #[test]
    fn test_resolve_sub_agent_completion_cancelled() {
        let mut waiting_on = Some("subagent_1".to_string());
        let mut sub_agent_sessions = vec!["subagent_1".to_string()];

        let resolution = resolve_sub_agent_completion(
            &mut waiting_on,
            &mut sub_agent_sessions,
            "subagent_1",
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
        assert!(sub_agent_sessions.is_empty());
    }

    #[test]
    fn test_resolve_sub_agent_completion_interrupted() {
        let mut waiting_on = Some("subagent_1".to_string());
        let mut sub_agent_sessions = vec!["subagent_1".to_string()];

        let resolution = resolve_sub_agent_completion(
            &mut waiting_on,
            &mut sub_agent_sessions,
            "subagent_1",
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
        assert!(sub_agent_sessions.is_empty());
    }

    #[test]
    fn test_resolve_sub_agent_completion_ignores_mismatch() {
        let mut waiting_on = Some("subagent_1".to_string());
        let mut sub_agent_sessions = vec!["subagent_1".to_string(), "subagent_2".to_string()];

        let resolution = resolve_sub_agent_completion(
            &mut waiting_on,
            &mut sub_agent_sessions,
            "subagent_2",
            &serde_json::json!({
                "status": "completed",
                "summary": "done"
            }),
        );

        assert!(resolution.is_none());
        assert_eq!(waiting_on, Some("subagent_1".to_string()));
        assert_eq!(
            sub_agent_sessions,
            vec!["subagent_1".to_string(), "subagent_2".to_string()]
        );
    }

    #[test]
    fn test_render_call_mode_sub_agent_tool_result_uses_xml_sections() {
        let output = render_call_mode_sub_agent_tool_result(
            "subagent_1",
            "completed",
            Some("Inspect OCR module"),
            "Found the issue",
            Some("OCR path confirmed"),
            true,
        );

        assert!(output.contains("reused=\"true\""));
        assert!(output.contains("<Task>\nInspect OCR module\n</Task>"));
        assert!(output.contains("<Result>\nFound the issue\n</Result>"));
        assert!(output.contains("<Summary>\nOCR path confirmed\n</Summary>"));
        assert!(!output.contains("Task:\n"));
        assert!(!output.contains("Result:\n"));
    }
}
