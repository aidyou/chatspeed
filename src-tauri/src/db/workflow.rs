//! Workflow database operations
//!
//! This module provides database operations for managing workflows and their messages.

use crate::db::{MainStore, StoreError};
use crate::workflow::react::events::{WorkflowEvent, WorkflowEventRecord};
use crate::workflow::react::types::ExecutionContext;
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// =================================================
//  Structs
// =================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub id: Option<String>,
    pub title: Option<String>,
    pub user_query: String,
    pub todo_list: Option<String>,
    #[serde(default = "default_workflow_status")]
    pub status: String,
    pub wait_reason: Option<String>,
    pub agent_id: String,
    pub agent_config: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

fn default_workflow_status() -> String {
    "pending".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowMessage {
    pub id: Option<i64>,
    pub session_id: String,
    pub role: String,
    pub message: String,
    pub reasoning: Option<String>,
    pub metadata: Option<Value>,
    pub attached_context: Option<String>,
    pub step_type: Option<String>,
    pub step_index: i32,
    #[serde(default)]
    pub is_error: bool,
    pub error_type: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSnapshot {
    pub workflow: Workflow,
    pub messages: Vec<WorkflowMessage>,
}

// =================================================
//  From Row Implementations
// =================================================

impl From<&Row<'_>> for Workflow {
    fn from(row: &Row<'_>) -> Self {
        Self {
            id: row.get("id").ok(),
            title: row.get("title").ok(),
            user_query: row.get("user_query").unwrap_or_default(),
            todo_list: row.get("todo_list").ok(),
            status: row.get("status").unwrap_or_else(|_| "pending".to_string()),
            wait_reason: row.get("wait_reason").ok(),
            agent_id: row.get("agent_id").unwrap_or_default(),
            agent_config: row.get("agent_config").ok(),
            created_at: row.get("created_at").ok(),
            updated_at: row.get("updated_at").ok(),
        }
    }
}

impl From<&Row<'_>> for WorkflowMessage {
    fn from(row: &Row<'_>) -> Self {
        let metadata_str: Option<String> = row.get("metadata").ok();
        let metadata = metadata_str.and_then(|s| serde_json::from_str(&s).ok());

        Self {
            id: row.get("id").ok(),
            session_id: row.get("session_id").unwrap_or_default(),
            role: row.get("role").unwrap_or_default(),
            message: row.get("message").unwrap_or_default(),
            reasoning: row.get("reasoning").ok(),
            metadata,
            attached_context: row.get("attached_context").ok(),
            step_type: row.get("step_type").ok(),
            step_index: row.get("step_index").unwrap_or_default(),
            is_error: row
                .get::<_, Option<i32>>("is_error")
                .map(|v| v == Some(1))
                .unwrap_or(false),
            error_type: row.get("error_type").ok(),
            created_at: row.get("created_at").ok(),
        }
    }
}

impl From<&Row<'_>> for WorkflowEventRecord {
    fn from(row: &Row<'_>) -> Self {
        let event_data_str: String = row.get("event_data").unwrap_or_default();
        let event_data: Value = serde_json::from_str(&event_data_str).unwrap_or(Value::Null);

        Self {
            id: row.get("id").unwrap_or_default(),
            session_id: row.get("session_id").unwrap_or_default(),
            event_type: row.get("event_type").unwrap_or_default(),
            event_version: row.get("event_version").unwrap_or_default(),
            event_data,
            created_at: row.get("created_at").unwrap_or_default(),
        }
    }
}

// =================================================
//  MainStore Implementation
// =================================================

impl MainStore {
    pub fn create_workflow(
        &self,
        id: &str,
        user_query: &str,
        agent_id: &str,
        agent_config: Option<String>,
    ) -> Result<Workflow, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        conn.execute(
            "INSERT INTO workflows (id, user_query, agent_id, agent_config, status) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, user_query, agent_id, agent_config, "pending"],
        )?;

        let workflow: Workflow = conn.query_row(
            "SELECT * FROM workflows WHERE id = ?1",
            params![id],
            |row| Ok(Workflow::from(row)),
        )?;

        Ok(workflow)
    }

    pub fn list_workflows(&self) -> Result<Vec<Workflow>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let mut stmt = conn.prepare("SELECT * FROM workflows ORDER BY created_at DESC")?;
        let rows = stmt.query_map([], |row| Ok(Workflow::from(row)))?;
        let mut workflows = Vec::new();
        for row in rows {
            workflows.push(row?);
        }
        Ok(workflows)
    }

    pub fn delete_workflow(&self, id: &str) -> Result<(), StoreError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let tx = conn.transaction()?;

        // 1. Delete associated messages first
        tx.execute(
            "DELETE FROM workflow_messages WHERE session_id = ?1",
            params![id],
        )?;

        // 2. Delete snapshot (stage 2)
        tx.execute(
            "DELETE FROM workflow_snapshots WHERE session_id = ?1",
            params![id],
        )?;

        // 3. Delete events (stage 4)
        tx.execute(
            "DELETE FROM workflow_events WHERE session_id = ?1",
            params![id],
        )?;

        // 4. Delete the workflow record
        tx.execute("DELETE FROM workflows WHERE id = ?1", params![id])?;

        tx.commit()?;
        Ok(())
    }

    pub fn get_workflow_snapshot(&self, id: &str) -> Result<WorkflowSnapshot, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let mut workflow: Workflow = conn.query_row(
            "SELECT * FROM workflows WHERE id = ?1",
            params![id],
            |row| Ok(Workflow::from(row)),
        )?;

        // Get wait_reason from workflow_snapshots table
        let wait_reason: Option<String> = conn
            .query_row(
                "SELECT wait_reason FROM workflow_snapshots WHERE session_id = ?1",
                params![id],
                |row| row.get(0),
            )
            .ok();
        workflow.wait_reason = wait_reason;

        let mut stmt =
            conn.prepare("SELECT * FROM workflow_messages WHERE session_id = ?1 ORDER BY id ASC")?;
        let messages_iter = stmt.query_map(params![id], |row| Ok(WorkflowMessage::from(row)))?;
        let mut messages = Vec::new();
        for msg in messages_iter {
            messages.push(msg?);
        }

        Ok(WorkflowSnapshot { workflow, messages })
    }

    pub fn add_workflow_message(
        &self,
        msg: &WorkflowMessage,
    ) -> Result<WorkflowMessage, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let metadata_json = msg
            .metadata
            .as_ref()
            .map(|m| serde_json::to_string(m).unwrap_or_default());

        conn.execute(
            "INSERT INTO workflow_messages (session_id, role, message, reasoning, metadata, attached_context, step_type, step_index, is_error, error_type)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                msg.session_id,
                msg.role,
                msg.message,
                msg.reasoning,
                metadata_json,
                msg.attached_context,
                msg.step_type,
                msg.step_index,
                if msg.is_error { 1 } else { 0 },
                msg.error_type,
            ],
        )?;

        let id = conn.last_insert_rowid();
        let mut new_msg = msg.clone();
        new_msg.id = Some(id);
        Ok(new_msg)
    }

    pub fn update_workflow_status(&self, id: &str, status: &str) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflows SET status = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
            params![status, id],
        )?;
        Ok(())
    }

    pub fn update_workflow_title(&self, id: &str, title: &str) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflows SET title = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
            params![title, id],
        )?;
        Ok(())
    }

    pub fn update_workflow_title_and_query(
        &self,
        id: &str,
        title: &str,
        user_query: &str,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflows SET title = ?1, user_query = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?3",
            params![title, user_query, id],
        )?;
        Ok(())
    }

    pub fn update_workflow_todo_list(&self, id: &str, todo_list: &str) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflows SET todo_list = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
            params![todo_list, id],
        )?;
        Ok(())
    }

    pub fn update_workflow_agent_config(
        &self,
        id: &str,
        agent_config: &str,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflows SET agent_config = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
            params![agent_config, id],
        )?;
        Ok(())
    }

    pub fn get_todo_list_for_workflow(&self, id: &str) -> Result<Vec<Value>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let todo_list_str: Option<String> = conn
            .query_row(
                "SELECT todo_list FROM workflows WHERE id = ?1",
                params![id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();

        Ok(todo_list_str
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default())
    }

    pub fn delete_workflow_messages(
        &self,
        session_id: &str,
        keep_ids: Vec<i64>,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        if keep_ids.is_empty() {
            conn.execute(
                "DELETE FROM workflow_messages WHERE session_id = ?1",
                params![session_id],
            )?;
        } else {
            let id_list = keep_ids
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let query = format!(
                "DELETE FROM workflow_messages WHERE session_id = ?1 AND id NOT IN ({})",
                id_list
            );
            conn.execute(&query, params![session_id])?;
        }
        Ok(())
    }

    // ExecutionContext Snapshot Operations

    pub fn get_execution_context(
        &self,
        session_id: &str,
    ) -> Result<Option<ExecutionContext>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let result: Option<String> = conn
            .query_row(
                "SELECT context_json FROM workflow_snapshots WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?;

        match result {
            Some(context_json) => {
                let ctx: ExecutionContext = serde_json::from_str(&context_json)?;
                log::info!(
                    "[Workflow][session={}] snapshot.read - state={:?}, wait_reason={:?}, pending_tools={}",
                    session_id,
                    ctx.state,
                    ctx.wait_reason,
                    ctx.pending_tools.len()
                );
                Ok(Some(ctx))
            }
            None => Ok(None),
        }
    }

    pub fn upsert_execution_context(&self, ctx: &ExecutionContext) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let context_json = serde_json::to_string(ctx)?;
        let state_str = ctx.state.to_string();
        let wait_reason_str = ctx.wait_reason.as_ref().map(|wr| wr.to_string());

        conn.execute(
            "INSERT OR REPLACE INTO workflow_snapshots (session_id, context_json, version, state, wait_reason, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP)",
            params![
                ctx.session_id,
                context_json,
                ctx.version,
                state_str,
                wait_reason_str,
            ],
        )?;

        log::info!(
            "[Workflow][session={}] snapshot.write - state={:?}, wait_reason={:?}, pending_tools={}",
            ctx.session_id,
            ctx.state,
            ctx.wait_reason,
            ctx.pending_tools.len()
        );

        Ok(())
    }

    pub fn delete_execution_context(&self, session_id: &str) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        conn.execute(
            "DELETE FROM workflow_snapshots WHERE session_id = ?1",
            params![session_id],
        )?;

        log::info!("[Workflow][session={}] snapshot.deleted", session_id);

        Ok(())
    }

    // Workflow Event Operations

    pub fn append_workflow_event(&self, event: &WorkflowEvent) -> Result<i64, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let event_type_str = event.event_type.as_str().to_string();
        let event_data_str = serde_json::to_string(&event.event_data)?;

        conn.execute(
            "INSERT INTO workflow_events (session_id, event_type, event_version, event_data)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                event.session_id,
                event_type_str,
                event.version,
                event_data_str
            ],
        )?;

        let event_id = conn.last_insert_rowid();

        log::info!(
            "[Workflow][session={}] event.append - type={:?}, event_id={}",
            event.session_id,
            event.event_type,
            event_id
        );

        Ok(event_id)
    }

    pub fn list_workflow_events(
        &self,
        session_id: &str,
    ) -> Result<Vec<WorkflowEventRecord>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let mut stmt = conn.prepare(
            "SELECT id, session_id, event_type, event_version, event_data, created_at
             FROM workflow_events
             WHERE session_id = ?1
             ORDER BY id ASC",
        )?;

        let rows = stmt.query_map(params![session_id], |row| {
            Ok(WorkflowEventRecord::from(row))
        })?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }

        log::info!(
            "[Workflow][session={}] event.list - count={}",
            session_id,
            events.len()
        );

        Ok(events)
    }

    pub fn get_last_event_id(&self, session_id: &str) -> Result<Option<i64>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let result: Option<i64> = conn
            .query_row(
                "SELECT MAX(id) FROM workflow_events WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::types::{RuntimeState, WaitReason};
    use tempfile::tempdir;

    fn create_test_store() -> MainStore {
        let dir = tempdir().expect("failed to create temp dir");
        let db_path = dir.path().join("workflow_phase4_test.db");
        MainStore::new(db_path).expect("failed to create MainStore")
    }

    #[test]
    fn test_workflow_events_table_and_index_exist_after_migration() {
        let store = create_test_store();
        let conn = store
            .conn
            .lock()
            .expect("failed to lock db connection for schema check");

        let table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM sqlite_master WHERE type = 'table' AND name = 'workflow_events'",
                [],
                |row| row.get(0),
            )
            .expect("failed to query workflow_events table existence");
        assert_eq!(table_exists, 1, "workflow_events table should exist");

        let index_exists: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM sqlite_master WHERE type = 'index' AND name = 'idx_workflow_events_session_id_id'",
                [],
                |row| row.get(0),
            )
            .expect("failed to query workflow_events index existence");
        assert_eq!(
            index_exists, 1,
            "idx_workflow_events_session_id_id index should exist"
        );
    }

    #[test]
    fn test_append_workflow_event_returns_error_when_table_missing() {
        let store = create_test_store();
        {
            let conn = store
                .conn
                .lock()
                .expect("failed to lock db connection for table drop");
            conn.execute("DROP TABLE workflow_events", [])
                .expect("failed to drop workflow_events table");
        }

        let event = WorkflowEvent::workflow_started(
            "session-append-fail".to_string(),
            "agent-test".to_string(),
        );
        let result = store.append_workflow_event(&event);
        assert!(
            result.is_err(),
            "append_workflow_event should return error when table is missing"
        );
    }

    #[test]
    fn test_snapshot_last_event_id_aligns_with_event_tail_for_key_states() {
        let store = create_test_store();
        let session_id = "session-last-event-align";

        let started =
            WorkflowEvent::workflow_started(session_id.to_string(), "agent-test".to_string());
        let e1 = store
            .append_workflow_event(&started)
            .expect("failed to append workflow_started event");
        let state_changed = WorkflowEvent::state_changed(
            session_id.to_string(),
            "thinking".to_string(),
            "executing".to_string(),
        );
        let e2 = store
            .append_workflow_event(&state_changed)
            .expect("failed to append state_changed event");
        assert!(e2 >= e1, "event ids should be monotonic");

        let expected_last_event_id = store
            .get_last_event_id(session_id)
            .expect("failed to query last event id")
            .expect("last event id should exist after appends");

        let mut waiting_ctx = ExecutionContext::new(session_id.to_string());
        waiting_ctx.state = RuntimeState::Waiting;
        waiting_ctx.wait_reason = Some(WaitReason::Approval);
        waiting_ctx.last_event_id = Some(expected_last_event_id);
        store
            .upsert_execution_context(&waiting_ctx)
            .expect("failed to save waiting snapshot");
        let loaded_waiting = store
            .get_execution_context(session_id)
            .expect("failed to load waiting snapshot")
            .expect("waiting snapshot should exist");
        assert_eq!(
            loaded_waiting.last_event_id,
            Some(expected_last_event_id),
            "waiting snapshot last_event_id should align with event tail"
        );

        let mut completed_ctx = waiting_ctx.clone();
        completed_ctx.state = RuntimeState::Completed;
        completed_ctx.wait_reason = None;
        completed_ctx.last_event_id = Some(expected_last_event_id);
        store
            .upsert_execution_context(&completed_ctx)
            .expect("failed to save completed snapshot");
        let loaded_completed = store
            .get_execution_context(session_id)
            .expect("failed to load completed snapshot")
            .expect("completed snapshot should exist");
        assert_eq!(
            loaded_completed.last_event_id,
            Some(expected_last_event_id),
            "completed snapshot last_event_id should align with event tail"
        );

        let mut cancelled_ctx = completed_ctx.clone();
        cancelled_ctx.state = RuntimeState::Cancelled;
        cancelled_ctx.last_event_id = Some(expected_last_event_id);
        store
            .upsert_execution_context(&cancelled_ctx)
            .expect("failed to save cancelled snapshot");
        let loaded_cancelled = store
            .get_execution_context(session_id)
            .expect("failed to load cancelled snapshot")
            .expect("cancelled snapshot should exist");
        assert_eq!(
            loaded_cancelled.last_event_id,
            Some(expected_last_event_id),
            "cancelled snapshot last_event_id should align with event tail"
        );
    }
}
