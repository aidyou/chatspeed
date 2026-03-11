//! Workflow database operations
//!
//! This module provides database operations for managing workflows and their messages.

use crate::db::{MainStore, StoreError};
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// =================================================
//  Structs
// =================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub id: String,
    pub title: Option<String>,
    pub user_query: String,
    pub todo_list: Option<String>,
    pub status: String,
    pub agent_id: String,
    pub allowed_paths: Option<Value>,
    pub final_audit: Option<bool>,
    pub created_at: String,
    pub updated_at: String,
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
        let allowed_paths: Option<Value> = row
            .get::<_, Option<String>>("allowed_paths")
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok());

        Self {
            id: row.get("id").unwrap_or_default(),
            title: row.get("title").ok(),
            user_query: row.get("user_query").unwrap_or_default(),
            todo_list: row.get("todo_list").ok(),
            status: row.get("status").unwrap_or_else(|_| "pending".to_string()),
            agent_id: row.get("agent_id").unwrap_or_default(),
            allowed_paths,
            final_audit: row.get("final_audit").ok(),
            created_at: row.get("created_at").unwrap_or_default(),
            updated_at: row.get("updated_at").unwrap_or_default(),
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

// =================================================
//  MainStore Implementation
// =================================================

impl MainStore {
    pub fn create_workflow(
        &self,
        id: &str,
        user_query: &str,
        agent_id: &str,
        allowed_paths: Option<Value>,
        final_audit: Option<bool>,
    ) -> Result<Workflow, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let allowed_paths_json =
            allowed_paths.map(|v| serde_json::to_string(&v).unwrap_or_default());

        conn.execute(
            "INSERT INTO workflows (id, user_query, agent_id, allowed_paths, final_audit, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, user_query, agent_id, allowed_paths_json, final_audit.unwrap_or(false), "pending"],
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
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "DELETE FROM workflow_messages WHERE session_id = ?1",
            params![id],
        )?;
        conn.execute("DELETE FROM workflows WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn get_workflow_snapshot(&self, id: &str) -> Result<WorkflowSnapshot, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let workflow: Workflow = conn.query_row(
            "SELECT * FROM workflows WHERE id = ?1",
            params![id],
            |row| Ok(Workflow::from(row)),
        )?;

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

    pub fn update_workflow_allowed_paths(
        &self,
        id: &str,
        paths_json: &str,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflows SET allowed_paths = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
            params![paths_json, id],
        )?;
        Ok(())
    }

    pub fn update_workflow_final_audit(
        &self,
        id: &str,
        final_audit: bool,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflows SET final_audit = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
            params![final_audit, id],
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
                |row| row.get(0),
            )
            .optional()?;

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
}
