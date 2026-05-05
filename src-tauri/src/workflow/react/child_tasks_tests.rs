#![cfg(test)]

use crate::db::MainStore;
use crate::workflow::react::child_tasks::{
    get_sub_agent_registry, resolve_sub_agent_completion, SubAgentRegistry,
};
use crate::workflow::react::manager::WorkflowManager;
use crate::workflow::react::types::{ExecutionContext, RuntimeState, WaitReason};
use std::sync::Arc;
use tempfile::tempdir;

fn create_test_store() -> Arc<std::sync::RwLock<MainStore>> {
    let dir = tempdir().expect("failed to create temp dir");
    let db_path = dir.path().join("test.db");
    Arc::new(std::sync::RwLock::new(
        MainStore::new(&db_path).expect("failed to create store"),
    ))
}

#[test]
fn test_p0_parent_session_enters_waiting_on_sub_agent() {
    let mut ctx = ExecutionContext::new("parent_session".to_string());

    ctx.wait_reason = Some(WaitReason::SubAgent);
    ctx.state = RuntimeState::Waiting;
    ctx.waiting_on_sub_agent_id = Some("subagent_1".to_string());
    ctx.sub_agent_sessions.push("subagent_1".to_string());

    assert_eq!(ctx.waiting_on_sub_agent_id, Some("subagent_1".to_string()));
    assert_eq!(ctx.state, RuntimeState::Waiting);
    assert_eq!(ctx.wait_reason, Some(WaitReason::SubAgent));
}

#[test]
fn test_p0_sub_agent_completion_triggers_parent_resume() {
    let mut waiting_on = Some("subagent_1".to_string());
    let mut sub_agent_sessions = vec!["subagent_1".to_string()];

    let resolution = resolve_sub_agent_completion(
        &mut waiting_on,
        &mut sub_agent_sessions,
        "subagent_1",
        &serde_json::json!({
            "status": "completed",
            "summary": "sub-agent done"
        }),
    )
    .unwrap();

    assert_eq!(resolution.status, "completed");
    assert_eq!(resolution.content, "sub-agent done");
    assert!(!resolution.is_error);
    assert_eq!(waiting_on, None);
    assert!(sub_agent_sessions.is_empty());
}

#[test]
fn test_p0_sub_agent_failure_triggers_parent_convergence() {
    let mut waiting_on = Some("subagent_1".to_string());
    let mut sub_agent_sessions = vec!["subagent_1".to_string()];

    let resolution = resolve_sub_agent_completion(
        &mut waiting_on,
        &mut sub_agent_sessions,
        "subagent_1",
        &serde_json::json!({
            "status": "failed",
            "error": "tool failed"
        }),
    )
    .unwrap();

    assert_eq!(resolution.status, "failed");
    assert_eq!(resolution.content, "tool failed");
    assert!(resolution.is_error);
    assert_eq!(waiting_on, None);
    assert!(sub_agent_sessions.is_empty());
}

#[test]
fn test_p0_sub_agent_cancelled_triggers_parent_convergence() {
    let mut waiting_on = Some("subagent_1".to_string());
    let mut sub_agent_sessions = vec!["subagent_1".to_string()];

    let resolution = resolve_sub_agent_completion(
        &mut waiting_on,
        &mut sub_agent_sessions,
        "subagent_1",
        &serde_json::json!({
            "status": "cancelled",
            "error": "cancelled"
        }),
    )
    .unwrap();

    assert_eq!(resolution.status, "cancelled");
    assert_eq!(resolution.content, "cancelled");
    assert!(resolution.is_error);
    assert_eq!(waiting_on, None);
    assert!(sub_agent_sessions.is_empty());
}

#[test]
fn test_p0_parent_knows_waiting_task_after_restart() {
    let store = create_test_store();

    let mut ctx = ExecutionContext::new("parent_session".to_string());
    ctx.wait_reason = Some(WaitReason::SubAgent);
    ctx.state = RuntimeState::Waiting;
    ctx.waiting_on_sub_agent_id = Some("subagent_1".to_string());
    ctx.sub_agent_sessions = vec!["subagent_1".to_string(), "subagent_2".to_string()];

    let store_guard = store.write().unwrap();
    store_guard.upsert_execution_context(&ctx).unwrap();
    drop(store_guard);

    let store_guard = store.read().unwrap();
    let loaded = store_guard.get_execution_context("parent_session").unwrap();

    assert!(loaded.is_some());
    let loaded_ctx = loaded.unwrap();
    assert_eq!(
        loaded_ctx.waiting_on_sub_agent_id,
        Some("subagent_1".to_string())
    );
    assert_eq!(loaded_ctx.sub_agent_sessions.len(), 2);
    assert!(loaded_ctx
        .sub_agent_sessions
        .contains(&"subagent_1".to_string()));
    assert!(loaded_ctx
        .sub_agent_sessions
        .contains(&"subagent_2".to_string()));
}

#[test]
fn test_p0_execution_context_serialization_with_sub_agent() {
    let mut ctx = ExecutionContext::new("parent_session".to_string());

    ctx.wait_reason = Some(WaitReason::SubAgent);
    ctx.state = RuntimeState::Waiting;
    ctx.waiting_on_sub_agent_id = Some("subagent_1".to_string());
    ctx.sub_agent_sessions.push("subagent_1".to_string());

    let json = serde_json::to_string(&ctx).unwrap();
    let restored: ExecutionContext = serde_json::from_str(&json).unwrap();

    assert_eq!(
        restored.waiting_on_sub_agent_id,
        Some("subagent_1".to_string())
    );
    assert_eq!(restored.sub_agent_sessions.len(), 1);
    assert_eq!(restored.sub_agent_sessions[0], "subagent_1");
    assert_eq!(restored.wait_reason, Some(WaitReason::SubAgent));
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

    assert_eq!(
        registry
            .get_parent_info("subagent_1")
            .unwrap()
            .parent_session_id,
        "parent_1"
    );
    assert_eq!(
        registry
            .get_parent_info("subagent_2")
            .unwrap()
            .parent_session_id,
        "parent_1"
    );
    assert_eq!(
        registry
            .get_parent_info("subagent_3")
            .unwrap()
            .parent_session_id,
        "parent_2"
    );

    let parent_1_children = registry.list_sub_agents_for_parent("parent_1");
    assert_eq!(parent_1_children.len(), 2);
    assert!(parent_1_children.contains(&"subagent_1".to_string()));
    assert!(parent_1_children.contains(&"subagent_2".to_string()));
}

#[test]
fn test_global_sub_agent_registry_singleton() {
    let registry1 = get_sub_agent_registry();
    let registry2 = get_sub_agent_registry();

    registry1.register_sub_agent("subagent_test".to_string(), "test_parent".to_string());

    assert!(registry2.is_sub_agent("subagent_test"));
    assert_eq!(
        registry2.list_sub_agents_for_parent("test_parent"),
        vec!["subagent_test".to_string()]
    );

    registry1.unregister_sub_agent("subagent_test");
}

#[test]
fn test_workflow_manager_session_lifecycle() {
    let manager = WorkflowManager::new();
    let session_id = "parent_session".to_string();

    assert!(!manager.has_session(&session_id));
    assert_eq!(manager.session_count(), 0);
}
