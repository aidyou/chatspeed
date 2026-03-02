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
        // Try to get as string first, then parse JSON
        let allowed_paths: Option<Value> = row.get::<_, Option<String>>("allowed_paths")
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
            created_at: row.get("created_at").unwrap_or_default(),
            updated_at: row.get("updated_at").unwrap_or_default(),
        }
    }
}

impl From<&Row<'_>> for WorkflowMessage {
    fn from(row: &Row<'_>) -> Self {
        let metadata_str: Option<String> = row.get("metadata").ok();
        let metadata = metadata_str.and_then(|s| {
            serde_json::from_str(&s)
                .map_err(|e| {
                    log::warn!(
                        "Failed to parse metadata JSON for AI Model (id: {:?}): {}, error: {}",
                        row.get::<_, Option<i64>>("id").unwrap_or_default(),
                        s,
                        e
                    );
                    e
                })
                .ok()
        });

        Self {
            id: row.get("id").ok(),
            session_id: row.get("session_id").unwrap_or_default(),
            role: row.get("role").unwrap_or_default(),
            message: row.get("message").unwrap_or_default(),
            reasoning: row.get("reasoning").ok(),
            metadata,
            step_type: row.get("step_type").ok(),
            step_index: row.get("step_index").unwrap_or_default(),
            is_error: row.get("is_error").unwrap_or(false),
            error_type: row.get("error_type").ok(),
            created_at: row.get("created_at").ok(),
        }
    }
}

// =================================================
//  MainStore Implementation
// =================================================

impl MainStore {
    /// Creates a new workflow in the database.
    pub fn create_workflow(
        &self,
        id: &str,
        user_query: &str,
        agent_id: &str,
        allowed_paths: Option<Value>,
    ) -> Result<Workflow, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let allowed_paths_str = allowed_paths.and_then(|v| serde_json::to_string(&v).ok());

        conn.execute(
            "INSERT INTO workflows (id, user_query, agent_id, status, allowed_paths) VALUES (?1, ?2, ?3, 'running', ?4)",
            params![&id, user_query, agent_id, &allowed_paths_str],
        )?;

        let workflow = conn.query_row(
            "SELECT * FROM workflows WHERE id = ?1",
            params![&id],
            |row| Ok(Workflow::from(row)),
        )?;

        Ok(workflow)
    }

    /// Adds a message to a workflow and returns the created message.
    pub fn add_workflow_message(
        &self,
        msg: &WorkflowMessage,
    ) -> Result<WorkflowMessage, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let metadata = msg
            .metadata
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());

        conn.execute(
            "INSERT INTO workflow_messages (session_id, role, message, reasoning, metadata, step_type, step_index, is_error, error_type) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![&msg.session_id, &msg.role, &msg.message, &msg.reasoning, &metadata, &msg.step_type, &msg.step_index, &msg.is_error, &msg.error_type],
        )?;

        let new_id = conn.last_insert_rowid();

        let new_msg = conn.query_row(
            "SELECT * FROM workflow_messages WHERE id = ?1",
            params![new_id],
            |row| Ok(WorkflowMessage::from(row)),
        )?;

        Ok(new_msg)
    }

    /// Gets a full snapshot of a workflow, including its messages.
    pub fn get_workflow_snapshot(&self, workflow_id: &str) -> Result<WorkflowSnapshot, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let workflow: Workflow = conn
            .query_row(
                "SELECT * FROM workflows WHERE id = ?1",
                params![workflow_id],
                |row| Ok(Workflow::from(row)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(workflow_id.to_string()))?;

        let mut stmt =
            conn.prepare("SELECT * FROM workflow_messages WHERE session_id = ?1 ORDER BY id ASC")?;
        let messages = stmt
            .query_map(params![workflow_id], |row| Ok(WorkflowMessage::from(row)))?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(WorkflowSnapshot { workflow, messages })
    }

    /// Lists all workflows.
    pub fn list_workflows(&self) -> Result<Vec<Workflow>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let mut stmt = conn.prepare("SELECT * FROM workflows ORDER BY updated_at DESC")?;
        let workflows = stmt
            .query_map(params![], |row| Ok(Workflow::from(row)))?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(workflows)
    }

    /// Deletes a workflow and all its messages.
    pub fn delete_workflow(&self, workflow_id: &str) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        // The ON DELETE CASCADE constraint will automatically delete all related messages
        conn.execute("DELETE FROM workflows WHERE id = ?1", params![workflow_id])?;

        Ok(())
    }

    /// Updates the allowed paths of a workflow.
    pub fn update_workflow_allowed_paths(
        &self,
        workflow_id: &str,
        allowed_paths: &str,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        conn.execute(
            "UPDATE workflows SET allowed_paths = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
            params![allowed_paths, workflow_id],
        )?;

        Ok(())
    }

    /// Updates the status of a workflow.
    pub fn update_workflow_status(
        &self,
        workflow_id: &str,
        status: &str,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        conn.execute(
            "UPDATE workflows SET status = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
            params![status, workflow_id],
        )?;

        Ok(())
    }

    /// Updates the title of a workflow.
    pub fn update_workflow_title(
        &self,
        workflow_id: &str,
        title: &str,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        conn.execute(
            "UPDATE workflows SET title = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
            params![title, workflow_id],
        )?;

        Ok(())
    }

    /// Updates the todo list of a workflow.
    pub fn update_workflow_todo_list(
        &self,
        workflow_id: &str,
        todo_list: &str,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        conn.execute(
            "UPDATE workflows SET todo_list = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
            params![todo_list, workflow_id],
        )?;

        Ok(())
    }

    /// Gets the todo list for a workflow as a JSON array.
    pub fn get_todo_list_for_workflow(
        &self,
        workflow_id: &str,
    ) -> Result<Vec<Value>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let todo_list_str: Option<String> = conn
            .query_row(
                "SELECT todo_list FROM workflows WHERE id = ?1",
                params![workflow_id],
                |row| row.get(0),
            )
            .optional()?;

        let todos = todo_list_str
            .and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok())
            .unwrap_or_default();

        Ok(todos)
    }
}
