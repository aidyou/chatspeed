#![cfg(test)]

use crate::db::MainStore;
use crate::workflow::react::child_tasks::{
    get_child_task_registry, resolve_child_task_completion, ChildTaskRegistry,
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
fn test_p0_parent_task_enters_waiting_on_task_id() {
    let mut ctx = ExecutionContext::new("parent_session".to_string());

    ctx.wait_reason = Some(WaitReason::ChildTask);
    ctx.state = RuntimeState::Waiting;
    ctx.waiting_on_task_id = Some("child_task_1".to_string());
    ctx.child_sessions.push("child_task_1".to_string());

    assert_eq!(ctx.waiting_on_task_id, Some("child_task_1".to_string()));
    assert_eq!(ctx.state, RuntimeState::Waiting);
    assert_eq!(ctx.wait_reason, Some(WaitReason::ChildTask));
}

#[test]
fn test_p0_child_task_completion_triggers_parent_resume() {
    let mut waiting_on = Some("child_1".to_string());
    let mut child_sessions = vec!["child_1".to_string()];

    let resolution = resolve_child_task_completion(
        &mut waiting_on,
        &mut child_sessions,
        "child_1",
        &serde_json::json!({
            "status": "completed",
            "summary": "child done"
        }),
    )
    .unwrap();

    assert_eq!(resolution.status, "completed");
    assert_eq!(resolution.content, "child done");
    assert!(!resolution.is_error);
    assert_eq!(waiting_on, None);
    assert!(child_sessions.is_empty());
}

#[test]
fn test_p0_child_task_failure_triggers_parent_convergence() {
    let mut waiting_on = Some("child_1".to_string());
    let mut child_sessions = vec!["child_1".to_string()];

    let resolution = resolve_child_task_completion(
        &mut waiting_on,
        &mut child_sessions,
        "child_1",
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
    assert!(child_sessions.is_empty());
}

#[test]
fn test_p0_child_task_cancelled_triggers_parent_convergence() {
    let mut waiting_on = Some("child_1".to_string());
    let mut child_sessions = vec!["child_1".to_string()];

    let resolution = resolve_child_task_completion(
        &mut waiting_on,
        &mut child_sessions,
        "child_1",
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
    assert!(child_sessions.is_empty());
}

#[test]
fn test_p0_parent_knows_waiting_task_after_restart() {
    let store = create_test_store();

    let mut ctx = ExecutionContext::new("parent_session".to_string());
    ctx.wait_reason = Some(WaitReason::ChildTask);
    ctx.state = RuntimeState::Waiting;
    ctx.waiting_on_task_id = Some("child_task_1".to_string());
    ctx.child_sessions = vec!["child_task_1".to_string(), "child_task_2".to_string()];

    let store_guard = store.write().unwrap();
    store_guard.upsert_execution_context(&ctx).unwrap();
    drop(store_guard);

    let store_guard = store.read().unwrap();
    let loaded = store_guard.get_execution_context("parent_session").unwrap();

    assert!(loaded.is_some());
    let loaded_ctx = loaded.unwrap();
    assert_eq!(
        loaded_ctx.waiting_on_task_id,
        Some("child_task_1".to_string())
    );
    assert_eq!(loaded_ctx.child_sessions.len(), 2);
    assert!(loaded_ctx
        .child_sessions
        .contains(&"child_task_1".to_string()));
    assert!(loaded_ctx
        .child_sessions
        .contains(&"child_task_2".to_string()));
}

#[test]
fn test_p0_execution_context_serialization_with_child_task() {
    let mut ctx = ExecutionContext::new("parent_session".to_string());

    ctx.wait_reason = Some(WaitReason::ChildTask);
    ctx.state = RuntimeState::Waiting;
    ctx.waiting_on_task_id = Some("child_task_1".to_string());
    ctx.child_sessions.push("child_task_1".to_string());

    let json = serde_json::to_string(&ctx).unwrap();
    let restored: ExecutionContext = serde_json::from_str(&json).unwrap();

    assert_eq!(
        restored.waiting_on_task_id,
        Some("child_task_1".to_string())
    );
    assert_eq!(restored.child_sessions.len(), 1);
    assert_eq!(restored.child_sessions[0], "child_task_1");
    assert_eq!(restored.wait_reason, Some(WaitReason::ChildTask));
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

    assert_eq!(
        registry
            .get_parent_info("child_1")
            .unwrap()
            .parent_session_id,
        "parent_1"
    );
    assert_eq!(
        registry
            .get_parent_info("child_2")
            .unwrap()
            .parent_session_id,
        "parent_1"
    );
    assert_eq!(
        registry
            .get_parent_info("child_3")
            .unwrap()
            .parent_session_id,
        "parent_2"
    );
}

#[test]
fn test_global_child_task_registry_singleton() {
    let registry1 = get_child_task_registry();
    let registry2 = get_child_task_registry();

    registry1.register_child_task("test_child".to_string(), "test_parent".to_string());

    assert!(registry2.is_child_task("test_child"));

    registry1.unregister_child_task("test_child");
}

#[test]
fn test_workflow_manager_session_lifecycle() {
    let manager = WorkflowManager::new();
    let session_id = "parent_session".to_string();

    assert!(!manager.has_session(&session_id));
    assert_eq!(manager.session_count(), 0);
}
