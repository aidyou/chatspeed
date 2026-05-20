//! Workflow database operations
//!
//! This module provides database operations for managing workflows and their messages.

use crate::db::{MainStore, StoreError};
use crate::workflow::react::events::{WorkflowEvent, WorkflowEventRecord};
use crate::workflow::react::types::ExecutionContext;
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

// =================================================
//  Structs
// =================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub id: Option<String>,
    pub parent_session_id: Option<String>,
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
    /// Durable workflow transcript history.
    /// This is the authoritative message record for audit, replay fallback,
    /// UI rendering, and semantic reporting.
    pub id: Option<i64>,
    pub session_id: String,
    pub role: String,
    pub message: String,
    pub reasoning: Option<String>,
    pub message_kind: String,
    pub message_subtype: Option<String>,
    pub segment_id: i32,
    pub source_event_type: Option<String>,
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
#[serde(rename_all = "camelCase")]
pub struct WorkflowAiContextMessage {
    /// AI-only projected context cache.
    /// This is derived from `WorkflowMessage` using explicit projection rules
    /// and exists only to feed the LLM efficiently.
    /// It is rebuildable and must not be used as recovery, UI, or reporting authority.
    pub id: Option<i64>,
    pub session_id: String,
    pub segment_id: i32,
    pub role: String,
    pub message: String,
    pub reasoning: Option<String>,
    pub message_kind: String,
    pub message_subtype: Option<String>,
    pub metadata: Option<Value>,
    pub source_message_id: Option<i64>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSnapshot {
    /// Snapshot payload returned to commands/UI.
    /// Transcript authority comes from `messages`; runtime recovery authority
    /// comes separately from `workflow_snapshots` via `ExecutionContext`.
    pub workflow: Workflow,
    pub messages: Vec<WorkflowMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEfficiencyMetrics {
    pub total_tool_calls: u32,
    pub search_calls: u32,
    pub read_calls: u32,
    pub edit_calls: u32,
    pub verification_calls: u32,
    pub no_match_searches: u32,
    pub parallel_search_rounds: u32,
    pub parallel_read_rounds: u32,
    pub repeated_read_files: u32,
    pub repeated_read_events: u32,
    pub batch_edit_rounds: u32,
    pub pre_edit_read_coverage: u32,
    pub convergence_score: u32,
    pub execution_score: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEfficiencySessionReport {
    pub session_id: String,
    pub parent_session_id: Option<String>,
    pub title: Option<String>,
    pub user_query: String,
    pub status: String,
    pub metrics: WorkflowEfficiencyMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEfficiencyReport {
    pub root_session_id: String,
    pub main_agent: WorkflowEfficiencySessionReport,
    pub sub_agents: Vec<WorkflowEfficiencySessionReport>,
}

// =================================================
//  From Row Implementations
// =================================================

impl From<&Row<'_>> for Workflow {
    fn from(row: &Row<'_>) -> Self {
        Self {
            id: row.get("id").ok(),
            parent_session_id: row.get("parent_session_id").ok(),
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
            message_kind: row
                .get("message_kind")
                .unwrap_or_else(|_| "message".to_string()),
            message_subtype: row.get("message_subtype").ok(),
            segment_id: row.get("segment_id").unwrap_or(1),
            source_event_type: row.get("source_event_type").ok(),
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

impl From<&Row<'_>> for WorkflowAiContextMessage {
    fn from(row: &Row<'_>) -> Self {
        let metadata_str: Option<String> = row.get("metadata").ok();
        let metadata = metadata_str.and_then(|s| serde_json::from_str(&s).ok());

        Self {
            id: row.get("id").ok(),
            session_id: row.get("session_id").unwrap_or_default(),
            segment_id: row.get("segment_id").unwrap_or_default(),
            role: row.get("role").unwrap_or_default(),
            message: row.get("message").unwrap_or_default(),
            reasoning: row.get("reasoning").ok(),
            message_kind: row
                .get("message_kind")
                .unwrap_or_else(|_| "message".to_string()),
            message_subtype: row.get("message_subtype").ok(),
            metadata,
            source_message_id: row.get("source_message_id").ok(),
            created_at: row.get("created_at").ok(),
        }
    }
}

// =================================================
//  MainStore Implementation
// =================================================

impl MainStore {
    pub fn get_workflow_efficiency_report(
        &self,
        session_id: &str,
    ) -> Result<WorkflowEfficiencyReport, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let root_workflow: Workflow = conn.query_row(
            "SELECT * FROM workflows WHERE id = ?1",
            params![session_id],
            |row| Ok(Workflow::from(row)),
        )?;

        let mut stmt = conn.prepare(
            "WITH RECURSIVE workflow_tree AS (
                SELECT *
                FROM workflows
                WHERE id = ?1
                UNION ALL
                SELECT workflows.*
                FROM workflows
                JOIN workflow_tree ON workflows.parent_session_id = workflow_tree.id
            )
            SELECT *
            FROM workflow_tree
            ORDER BY CASE WHEN id = ?1 THEN 0 ELSE 1 END, created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| Ok(Workflow::from(row)))?;

        let mut reports = Vec::new();
        for row in rows {
            let workflow = row?;
            let messages = self.list_workflow_messages_for_session_locked(
                &conn,
                workflow.id.as_deref().unwrap_or_default(),
            )?;
            let metrics = compute_efficiency_metrics(&messages);
            reports.push(WorkflowEfficiencySessionReport {
                session_id: workflow.id.clone().unwrap_or_default(),
                parent_session_id: workflow.parent_session_id.clone(),
                title: workflow.title.clone(),
                user_query: workflow.user_query.clone(),
                status: workflow.status.clone(),
                metrics,
            });
        }

        let main_agent = reports
            .iter()
            .find(|report| report.session_id == session_id)
            .cloned()
            .unwrap_or_else(|| WorkflowEfficiencySessionReport {
                session_id: session_id.to_string(),
                parent_session_id: root_workflow.parent_session_id.clone(),
                title: root_workflow.title.clone(),
                user_query: root_workflow.user_query.clone(),
                status: root_workflow.status.clone(),
                metrics: WorkflowEfficiencyMetrics::default(),
            });

        let sub_agents = reports
            .into_iter()
            .filter(|report| report.session_id != session_id)
            .collect();

        Ok(WorkflowEfficiencyReport {
            root_session_id: session_id.to_string(),
            main_agent,
            sub_agents,
        })
    }

    fn list_workflow_messages_for_session_locked(
        &self,
        conn: &rusqlite::Connection,
        session_id: &str,
    ) -> Result<Vec<WorkflowMessage>, StoreError> {
        let mut stmt = conn.prepare(
            "SELECT *
             FROM workflow_messages
             WHERE session_id = ?1
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| Ok(WorkflowMessage::from(row)))?;
        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        Ok(messages)
    }

    pub fn create_workflow(
        &self,
        id: &str,
        user_query: &str,
        agent_id: &str,
        agent_config: Option<String>,
        parent_session_id: Option<&str>,
    ) -> Result<Workflow, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        conn.execute(
            "INSERT INTO workflows (id, parent_session_id, user_query, agent_id, agent_config, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, parent_session_id, user_query, agent_id, agent_config, "pending"],
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
        let mut stmt = conn.prepare(
            "SELECT * FROM workflows
             WHERE parent_session_id IS NULL AND id NOT LIKE 'subagent\\_%' ESCAPE '\\'
             ORDER BY updated_at DESC, created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| Ok(Workflow::from(row)))?;
        let mut workflows = Vec::new();
        for row in rows {
            workflows.push(row?);
        }
        Ok(workflows)
    }

    pub fn list_nonterminal_child_workflows(&self) -> Result<Vec<Workflow>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT * FROM workflows
             WHERE parent_session_id IS NOT NULL
               AND LOWER(status) NOT IN ('completed', 'error', 'failed', 'cancelled')
             ORDER BY created_at ASC",
        )?;
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
        let workflow_ids = {
            let mut stmt = tx.prepare(
                "WITH RECURSIVE workflow_tree(id, depth) AS (
                    SELECT id, 0 FROM workflows WHERE id = ?1
                    UNION ALL
                    SELECT workflows.id, workflow_tree.depth + 1
                    FROM workflows
                    JOIN workflow_tree ON workflows.parent_session_id = workflow_tree.id
                )
                SELECT id FROM workflow_tree ORDER BY depth DESC",
            )?;
            let rows = stmt.query_map(params![id], |row| row.get::<_, String>(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            ids
        };

        for workflow_id in &workflow_ids {
            tx.execute(
                "DELETE FROM workflow_context_messages WHERE session_id = ?1",
                params![workflow_id],
            )?;

            tx.execute(
                "DELETE FROM workflow_messages WHERE session_id = ?1",
                params![workflow_id],
            )?;

            tx.execute(
                "DELETE FROM workflow_snapshots WHERE session_id = ?1",
                params![workflow_id],
            )?;

            // workflow_events is an audit/secondary table. If it is corrupted (e.g.,
            // "database disk image is malformed"), do not block the workflow deletion.
            // Log the error and continue so the primary workflow data still gets cleaned up.
            if let Err(e) = tx.execute(
                "DELETE FROM workflow_events WHERE session_id = ?1",
                params![workflow_id],
            ) {
                log::error!(
                    "[Workflow][session={}] Failed to delete workflow events (non-fatal, continuing): {}",
                    workflow_id,
                    e
                );
            }
        }

        // Delete child workflow rows before their parent to satisfy parent_session_id FK.
        for workflow_id in &workflow_ids {
            tx.execute("DELETE FROM workflows WHERE id = ?1", params![workflow_id])?;
        }

        tx.commit()?;
        Ok(())
    }

    pub fn delete_last_assistant_turn(&self, session_id: &str) -> Result<bool, StoreError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let tx = conn.transaction()?;

        let assistant_anchor_id: Option<i64> = tx
            .query_row(
                "SELECT id
                 FROM workflow_messages
                 WHERE session_id = ?1 AND role = 'assistant'
                 ORDER BY id DESC
                 LIMIT 1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?;

        let Some(assistant_anchor_id) = assistant_anchor_id else {
            return Ok(false);
        };

        // `workflow_context_messages` is an AI-only cache derived from transcript history.
        // After deleting a tail turn from durable history, partial cache surgery is brittle.
        // Clear the whole session cache and let the runtime rebuild it from authority data.
        tx.execute(
            "DELETE FROM workflow_context_messages WHERE session_id = ?1",
            params![session_id],
        )?;

        tx.execute(
            "DELETE FROM workflow_messages
             WHERE session_id = ?1 AND id >= ?2",
            params![session_id, assistant_anchor_id],
        )?;

        // Snapshots and events are recovery/audit projections. After deleting the tail
        // transcript turn, keeping them would allow stale state to resurrect the removed
        // approval/tool turn through replay or hot-resume.
        tx.execute(
            "DELETE FROM workflow_snapshots WHERE session_id = ?1",
            params![session_id],
        )?;
        tx.execute(
            "DELETE FROM workflow_events WHERE session_id = ?1",
            params![session_id],
        )?;

        tx.execute(
            "UPDATE workflows
             SET status = 'pending',
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![session_id],
        )?;

        tx.commit()?;
        Ok(true)
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
            "INSERT INTO workflow_messages (session_id, role, message, reasoning, message_kind, message_subtype, segment_id, source_event_type, metadata, attached_context, step_type, step_index, is_error, error_type)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                msg.session_id,
                msg.role,
                msg.message,
                msg.reasoning,
                msg.message_kind,
                msg.message_subtype,
                msg.segment_id,
                msg.source_event_type,
                metadata_json,
                msg.attached_context,
                msg.step_type,
                msg.step_index,
                if msg.is_error { 1 } else { 0 },
                msg.error_type,
            ],
        )?;

        conn.execute(
            "UPDATE workflows SET updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
            params![msg.session_id],
        )?;

        let id = conn.last_insert_rowid();
        let mut new_msg = msg.clone();
        new_msg.id = Some(id);
        Ok(new_msg)
    }

    pub fn update_workflow_message_metadata(
        &self,
        message_id: i64,
        metadata: &Value,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let metadata_json = serde_json::to_string(metadata).unwrap_or_default();

        conn.execute(
            "UPDATE workflow_messages SET metadata = ?1 WHERE id = ?2",
            params![metadata_json, message_id],
        )?;

        Ok(())
    }

    pub fn add_workflow_ai_context_message(
        &self,
        msg: &WorkflowAiContextMessage,
    ) -> Result<WorkflowAiContextMessage, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let metadata_json = msg
            .metadata
            .as_ref()
            .map(|m| serde_json::to_string(m).unwrap_or_default());

        conn.execute(
            "INSERT INTO workflow_context_messages (session_id, segment_id, role, message, reasoning, message_kind, message_subtype, metadata, source_message_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                msg.session_id,
                msg.segment_id,
                msg.role,
                msg.message,
                msg.reasoning,
                msg.message_kind,
                msg.message_subtype,
                metadata_json,
                msg.source_message_id,
            ],
        )?;

        let id = conn.last_insert_rowid();
        let mut new_msg = msg.clone();
        new_msg.id = Some(id);
        Ok(new_msg)
    }

    pub fn delete_workflow_ai_context_segment(
        &self,
        session_id: &str,
        segment_id: i32,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        conn.execute(
            "DELETE FROM workflow_context_messages WHERE session_id = ?1 AND segment_id = ?2",
            params![session_id, segment_id],
        )?;

        Ok(())
    }

    pub fn update_workflow_status(&self, id: &str, status: &str) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflows
             SET status = ?1,
                 updated_at = CASE
                     WHEN status IS NOT ?1 THEN CURRENT_TIMESTAMP
                     ELSE updated_at
                 END
             WHERE id = ?2",
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
            "UPDATE workflows
             SET title = ?1,
                 updated_at = CASE
                     WHEN title IS NOT ?1 THEN CURRENT_TIMESTAMP
                     ELSE updated_at
                 END
             WHERE id = ?2",
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
            "UPDATE workflows
             SET title = ?1,
                 user_query = ?2,
                 updated_at = CASE
                     WHEN title IS NOT ?1 OR user_query IS NOT ?2 THEN CURRENT_TIMESTAMP
                     ELSE updated_at
                 END
             WHERE id = ?3",
            params![title, user_query, id],
        )?;
        Ok(())
    }

    pub fn update_workflow_query(&self, id: &str, user_query: &str) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflows
             SET user_query = ?1,
                 updated_at = CASE
                     WHEN user_query IS NOT ?1 THEN CURRENT_TIMESTAMP
                     ELSE updated_at
                 END
             WHERE id = ?2",
            params![user_query, id],
        )?;
        Ok(())
    }

    pub fn update_workflow_todo_list(&self, id: &str, todo_list: &str) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflows
             SET todo_list = ?1,
                 updated_at = CASE
                     WHEN todo_list IS NOT ?1 THEN CURRENT_TIMESTAMP
                     ELSE updated_at
                 END
             WHERE id = ?2",
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
            "UPDATE workflows
             SET agent_config = ?1,
                 updated_at = CASE
                     WHEN agent_config IS NOT ?1 THEN CURRENT_TIMESTAMP
                     ELSE updated_at
                 END
             WHERE id = ?2",
            params![agent_config, id],
        )?;
        Ok(())
    }

    pub fn update_workflow_agent_id(&self, id: &str, agent_id: &str) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflows
             SET agent_id = ?1,
                 updated_at = CASE
                     WHEN agent_id IS NOT ?1 THEN CURRENT_TIMESTAMP
                     ELSE updated_at
                 END
             WHERE id = ?2",
            params![agent_id, id],
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
        let sub_agent_sessions_json = serde_json::to_string(&ctx.sub_agent_sessions)?;

        conn.execute(
            "INSERT OR REPLACE INTO workflow_snapshots
             (session_id, context_json, version, state, wait_reason, waiting_on_sub_agent_id, sub_agent_sessions, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CURRENT_TIMESTAMP)",
            params![
                ctx.session_id,
                context_json,
                ctx.version,
                state_str,
                wait_reason_str,
                ctx.waiting_on_sub_agent_id.clone(),
                sub_agent_sessions_json,
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

fn compute_efficiency_metrics(messages: &[WorkflowMessage]) -> WorkflowEfficiencyMetrics {
    let mut metrics = WorkflowEfficiencyMetrics::default();
    let mut read_counts: HashMap<String, u32> = HashMap::new();
    let mut seen_read_files: HashSet<String> = HashSet::new();
    let mut edited_files: HashSet<String> = HashSet::new();
    let mut read_before_edit_files: HashSet<String> = HashSet::new();

    for message in messages {
        if message.role == "assistant" {
            let tool_calls = extract_tool_calls_from_metadata(message.metadata.as_ref());
            let search_count = tool_calls
                .iter()
                .filter(|tool_call| is_search_tool(&tool_call.name))
                .count();
            let read_count = tool_calls
                .iter()
                .filter(|tool_call| is_read_tool(&tool_call.name))
                .count();
            let edit_count = tool_calls
                .iter()
                .filter(|tool_call| is_edit_tool(&tool_call.name))
                .count();

            if search_count >= 2 {
                metrics.parallel_search_rounds += 1;
            }
            if read_count >= 2 {
                metrics.parallel_read_rounds += 1;
            }
            if edit_count >= 2 {
                metrics.batch_edit_rounds += 1;
            }
            continue;
        }

        if message.role != "tool" {
            continue;
        }

        let tool_name = extract_tool_name(message.metadata.as_ref());
        if matches!(
            tool_name.as_deref(),
            Some("complete_workflow_with_summary" | "answer_user")
        ) {
            continue;
        }

        metrics.total_tool_calls += 1;

        if let Some(tool_name) = tool_name.as_deref() {
            if is_search_tool(tool_name) {
                metrics.search_calls += 1;
                if message.message.contains("[No matches found]") {
                    metrics.no_match_searches += 1;
                }
            }

            if is_read_tool(tool_name) {
                metrics.read_calls += 1;
                for path in extract_paths_for_tool(message, tool_name) {
                    let count = read_counts.entry(path.clone()).or_insert(0);
                    *count += 1;
                    seen_read_files.insert(path);
                }
            }

            if is_edit_tool(tool_name) {
                metrics.edit_calls += 1;
                for path in extract_paths_for_tool(message, tool_name) {
                    if seen_read_files.contains(&path) {
                        read_before_edit_files.insert(path.clone());
                    }
                    edited_files.insert(path);
                }
            }

            if is_verification_tool(tool_name, message.metadata.as_ref()) {
                metrics.verification_calls += 1;
            }
        }
    }

    metrics.repeated_read_files = read_counts.values().filter(|count| **count > 1).count() as u32;
    metrics.repeated_read_events = read_counts
        .values()
        .map(|count| count.saturating_sub(1))
        .sum();

    metrics.pre_edit_read_coverage = if edited_files.is_empty() {
        100
    } else {
        ((read_before_edit_files.len() as f64 / edited_files.len() as f64) * 100.0).round() as u32
    };

    metrics.convergence_score = score_convergence(&metrics);
    metrics.execution_score = score_execution(&metrics);

    metrics
}

#[derive(Debug, Clone)]
struct ToolCallShape {
    name: String,
}

fn extract_tool_calls_from_metadata(metadata: Option<&Value>) -> Vec<ToolCallShape> {
    metadata
        .and_then(|meta| meta.get("tool_calls"))
        .and_then(|tool_calls| tool_calls.as_array())
        .map(|tool_calls| {
            tool_calls
                .iter()
                .filter_map(|tool_call| {
                    tool_call
                        .get("function")
                        .and_then(|function| function.get("name"))
                        .and_then(|name| name.as_str())
                        .map(|name| ToolCallShape {
                            name: name.to_string(),
                        })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_tool_name(metadata: Option<&Value>) -> Option<String> {
    metadata
        .and_then(|meta| meta.get("tool_name"))
        .and_then(|tool_name| tool_name.as_str())
        .map(str::to_string)
        .or_else(|| {
            metadata
                .and_then(|meta| meta.get("tool_call"))
                .and_then(|tool_call| tool_call.get("function"))
                .and_then(|function| function.get("name"))
                .and_then(|name| name.as_str())
                .map(str::to_string)
        })
}

fn is_search_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "glob" | "grep" | "list_dir" | "search_workspace_files" | "web_search"
    )
}

fn is_read_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "read_file" | "read_git_base_text_file" | "web_fetch"
    )
}

fn is_edit_tool(tool_name: &str) -> bool {
    matches!(tool_name, "edit_file" | "write_file")
}

fn is_verification_tool(tool_name: &str, metadata: Option<&Value>) -> bool {
    if !matches!(tool_name, "bash" | "execute_command") {
        return false;
    }

    let Some(command_text) = metadata
        .and_then(|meta| meta.get("tool_call"))
        .and_then(|tool_call| tool_call.get("function"))
        .and_then(|function| function.get("arguments"))
        .and_then(|arguments| arguments.as_str())
    else {
        return false;
    };

    let command_text = command_text.to_ascii_lowercase();
    [
        "cargo check",
        "cargo test",
        "cargo clippy",
        "npm test",
        "pnpm test",
        "pnpm lint",
        "pnpm build",
        "go test",
        "pytest",
        "vitest",
        "jest",
        "ruff check",
    ]
    .iter()
    .any(|needle| command_text.contains(needle))
}

fn extract_paths_for_tool(message: &WorkflowMessage, tool_name: &str) -> Vec<String> {
    let mut paths = Vec::new();

    if matches!(tool_name, "read_file" | "read_git_base_text_file") {
        if let Some(path) = extract_file_content_path(&message.message) {
            paths.push(path);
        }
    }

    if let Some(arguments) = message
        .metadata
        .as_ref()
        .and_then(|meta| meta.get("tool_call"))
        .and_then(|tool_call| tool_call.get("function"))
        .and_then(|function| function.get("arguments"))
        .and_then(|arguments| arguments.as_str())
        .and_then(|arguments| serde_json::from_str::<Value>(arguments).ok())
    {
        collect_paths_from_json(&arguments, &mut paths);
    }

    paths.sort();
    paths.dedup();
    paths
}

fn extract_file_content_path(message: &str) -> Option<String> {
    let marker = "<file_content path=\"";
    let start = message.find(marker)? + marker.len();
    let rest = &message[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn collect_paths_from_json(value: &Value, output: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                let lower = key.to_ascii_lowercase();
                let should_collect = matches!(
                    lower.as_str(),
                    "file_path" | "path" | "relative_path" | "target_file"
                );
                if should_collect {
                    if let Some(path) = inner.as_str() {
                        output.push(path.to_string());
                    }
                }
                collect_paths_from_json(inner, output);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_paths_from_json(item, output);
            }
        }
        _ => {}
    }
}

fn score_convergence(metrics: &WorkflowEfficiencyMetrics) -> u32 {
    let mut score: i32 = 72;
    score += ((metrics.parallel_search_rounds.min(2)) as i32) * 4;
    score += ((metrics.parallel_read_rounds.min(2)) as i32) * 3;
    score -= ((metrics.no_match_searches.min(3)) as i32) * 5;
    score -= ((metrics.repeated_read_events.min(8)) as i32) * 2;

    if metrics.edit_calls > 0 || metrics.verification_calls > 0 {
        score += 6;
    }

    if metrics.total_tool_calls > 0 && metrics.search_calls == 0 && metrics.read_calls > 0 {
        score += 2;
    }

    score.clamp(35, 95) as u32
}

fn score_execution(metrics: &WorkflowEfficiencyMetrics) -> u32 {
    let mut score: i32 = 72;

    if metrics.edit_calls > 0 {
        score += ((metrics.pre_edit_read_coverage as i32) * 12) / 100;
        score += ((metrics.batch_edit_rounds.min(2)) as i32) * 3;
        if metrics.pre_edit_read_coverage < 50 {
            score -= 8;
        }
    }

    if metrics.verification_calls > 0 {
        score += 10;
    } else if metrics.edit_calls > 0 {
        score -= 10;
    }

    if metrics.edit_calls == 0 && metrics.verification_calls == 0 {
        score += 2;
    }

    score.clamp(45, 96) as u32
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

    #[test]
    fn test_list_workflows_excludes_child_workflows() {
        let store = create_test_store();

        store
            .create_workflow("parent-session", "Parent query", "agent-parent", None, None)
            .expect("failed to create parent workflow");
        store
            .create_workflow(
                "task_legacy_child_session",
                "Legacy child query",
                "agent-child",
                None,
                None,
            )
            .expect("failed to create legacy child workflow");
        store
            .create_workflow(
                "child-session",
                "Child query",
                "agent-child",
                None,
                Some("parent-session"),
            )
            .expect("failed to create child workflow");

        let workflows = store
            .list_workflows()
            .expect("failed to list top-level workflows");

        assert_eq!(workflows.len(), 1);
        assert_eq!(
            workflows[0].id.as_deref(),
            Some("parent-session"),
            "child workflow should not appear in top-level workflow list"
        );
    }

    #[test]
    fn test_delete_workflow_removes_sub_agent_descendants() {
        let store = create_test_store();

        store
            .create_workflow("parent-session", "Parent query", "agent-parent", None, None)
            .expect("failed to create parent workflow");
        store
            .create_workflow(
                "subagent-child",
                "Child query",
                "agent-child",
                None,
                Some("parent-session"),
            )
            .expect("failed to create child workflow");

        store
            .delete_workflow("parent-session")
            .expect("failed to recursively delete workflow tree");

        let conn = store
            .conn
            .lock()
            .expect("failed to lock db connection for delete assertion");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM workflows WHERE id IN ('parent-session', 'subagent-child')",
                [],
                |row| row.get(0),
            )
            .expect("failed to count deleted workflows");
        assert_eq!(
            count, 0,
            "parent and sub-agent workflow rows should be deleted"
        );
    }

    #[test]
    fn test_get_workflow_efficiency_report_splits_main_and_sub_agents() {
        let store = create_test_store();

        store
            .create_workflow("main-session", "Main task", "agent-main", None, None)
            .expect("failed to create main workflow");
        store
            .create_workflow(
                "subagent-child",
                "Explore task",
                "agent-child",
                None,
                Some("main-session"),
            )
            .expect("failed to create sub workflow");

        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: "main-session".to_string(),
                role: "assistant".to_string(),
                message: "Read and edit".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(serde_json::json!({
                    "tool_calls": [
                        {
                            "function": {
                                "name": "read_file",
                                "arguments": "{\"file_path\":\"src/main.rs\"}"
                            }
                        },
                        {
                            "function": {
                                "name": "read_file",
                                "arguments": "{\"file_path\":\"src/lib.rs\"}"
                            }
                        }
                    ]
                })),
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add main assistant context");
        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: "main-session".to_string(),
                role: "tool".to_string(),
                message: "<file_content path=\"src/main.rs\">fn main() {}</file_content>"
                    .to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(serde_json::json!({
                    "tool_name": "read_file",
                    "tool_call": {
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"file_path\":\"src/main.rs\"}"
                        }
                    }
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add main read tool");
        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: "main-session".to_string(),
                role: "tool".to_string(),
                message: "{\"file_path\":\"src/main.rs\"}".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(serde_json::json!({
                    "tool_name": "edit_file",
                    "tool_call": {
                        "function": {
                            "name": "edit_file",
                            "arguments": "{\"file_path\":\"src/main.rs\"}"
                        }
                    }
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 3,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add main edit tool");
        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: "main-session".to_string(),
                role: "tool".to_string(),
                message: "ok".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(serde_json::json!({
                    "tool_name": "bash",
                    "tool_call": {
                        "function": {
                            "name": "bash",
                            "arguments": "{\"command\":\"cargo check\"}"
                        }
                    }
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 4,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add main verification tool");

        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: "subagent-child".to_string(),
                role: "assistant".to_string(),
                message: "Search in parallel".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(serde_json::json!({
                    "tool_calls": [
                        {
                            "function": {
                                "name": "grep",
                                "arguments": "{\"pattern\":\"foo\"}"
                            }
                        },
                        {
                            "function": {
                                "name": "glob",
                                "arguments": "{\"pattern\":\"*.rs\"}"
                            }
                        }
                    ]
                })),
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add sub assistant context");
        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: "subagent-child".to_string(),
                role: "tool".to_string(),
                message: "[No matches found]".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(serde_json::json!({
                    "tool_name": "grep",
                    "tool_call": {
                        "function": {
                            "name": "grep",
                            "arguments": "{\"pattern\":\"foo\"}"
                        }
                    }
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add sub grep tool");

        let report = store
            .get_workflow_efficiency_report("main-session")
            .expect("failed to build efficiency report");

        assert_eq!(report.main_agent.session_id, "main-session");
        assert_eq!(report.sub_agents.len(), 1);
        assert_eq!(report.sub_agents[0].session_id, "subagent-child");
        assert_eq!(report.main_agent.metrics.read_calls, 1);
        assert_eq!(report.main_agent.metrics.edit_calls, 1);
        assert_eq!(report.main_agent.metrics.verification_calls, 1);
        assert_eq!(report.main_agent.metrics.pre_edit_read_coverage, 100);
        assert_eq!(report.sub_agents[0].metrics.parallel_search_rounds, 1);
        assert_eq!(report.sub_agents[0].metrics.no_match_searches, 1);
        assert!(report.main_agent.metrics.convergence_score >= 60);
        assert!(report.main_agent.metrics.execution_score >= 80);
    }

    #[test]
    fn test_delete_last_assistant_turn_removes_tail_messages_and_recovery_state() {
        let store = create_test_store();
        let session_id = "session-delete-last-assistant-turn";
        {
            let conn = store
                .conn
                .lock()
                .expect("failed to lock db connection for agent seed");
            conn.execute(
                "INSERT INTO agents (id, name, system_prompt, agent_type, max_contexts)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    "agent-test",
                    "Agent Test",
                    "You are a test agent.",
                    "autonomous",
                    20
                ],
            )
            .expect("failed to seed agent");
        }

        store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");

        let user_message = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "user".to_string(),
                message: "Fix it".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: None,
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add user message");

        let assistant_message = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "assistant".to_string(),
                message: "I'll edit the file".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(serde_json::json!({
                    "tool_calls": [{
                        "id": "tool_1",
                        "function": {
                            "name": "edit_file",
                            "arguments": "{\"file_path\":\"a.rs\"}"
                        }
                    }]
                })),
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add assistant message");

        let tool_message = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "tool".to_string(),
                message: "{\"file_path\":\"a.rs\"}".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(serde_json::json!({
                    "tool_call_id": "tool_1",
                    "tool_name": "edit_file",
                    "approval_status": "pending",
                    "execution_status": "pending_approval"
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 3,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add tool message");

        store
            .add_workflow_ai_context_message(&WorkflowAiContextMessage {
                id: None,
                session_id: session_id.to_string(),
                segment_id: 1,
                role: "assistant".to_string(),
                message: assistant_message.message.clone(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                metadata: assistant_message.metadata.clone(),
                source_message_id: assistant_message.id,
                created_at: None,
            })
            .expect("failed to add assistant context message");
        store
            .add_workflow_ai_context_message(&WorkflowAiContextMessage {
                id: None,
                session_id: session_id.to_string(),
                segment_id: 1,
                role: "tool".to_string(),
                message: tool_message.message.clone(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                metadata: tool_message.metadata.clone(),
                source_message_id: tool_message.id,
                created_at: None,
            })
            .expect("failed to add tool context message");

        let started =
            WorkflowEvent::workflow_started(session_id.to_string(), "agent-test".to_string());
        store
            .append_workflow_event(&started)
            .expect("failed to append workflow_started event");

        let mut context = ExecutionContext::new(session_id.to_string());
        context.state = RuntimeState::Waiting;
        context.wait_reason = Some(WaitReason::Approval);
        store
            .upsert_execution_context(&context)
            .expect("failed to persist snapshot");

        let deleted = store
            .delete_last_assistant_turn(session_id)
            .expect("failed to delete last assistant turn");
        assert!(deleted, "assistant tail should be deleted");

        let snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to load workflow snapshot after deletion");
        assert_eq!(snapshot.messages.len(), 1);
        assert_eq!(snapshot.messages[0].id, user_message.id);
        assert_eq!(snapshot.messages[0].role, "user");

        let conn = store
            .conn
            .lock()
            .expect("failed to lock db connection for assertions");

        let context_count: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM workflow_context_messages WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .expect("failed to count context messages");
        assert_eq!(context_count, 0);

        let snapshot_count: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM workflow_snapshots WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .expect("failed to count snapshots");
        assert_eq!(snapshot_count, 0);

        let event_count: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM workflow_events WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .expect("failed to count events");
        assert_eq!(event_count, 0);

        let status: String = conn
            .query_row(
                "SELECT status FROM workflows WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .expect("failed to read workflow status");
        assert_eq!(status, "pending");
    }
}
