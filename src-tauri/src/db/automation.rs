use crate::db::{MainStore, StoreError};
use rusqlite::{params, OptionalExtension, Row, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAutomation {
    pub id: String,
    pub title: String,
    pub prompt: Option<String>,
    pub prompt_file_path: Option<String>,
    pub agent_id: String,
    pub agent_config: Option<String>,
    pub allowed_paths: String,
    pub shell_config: Option<String>,
    pub schedule_kind: String,
    pub schedule_config: String,
    pub continuous_context: bool,
    pub current_workflow_session_id: Option<String>,
    pub self_review: bool,
    pub enabled: bool,
    pub next_run_at: Option<String>,
    pub last_run_at: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAutomationRun {
    pub id: String,
    pub automation_id: String,
    pub workflow_session_id: Option<String>,
    pub status: String,
    pub scheduled_for: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub error: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkflowAutomationUpsert {
    pub id: String,
    pub title: String,
    pub prompt: Option<String>,
    pub prompt_file_path: Option<String>,
    pub agent_id: String,
    pub agent_config: Option<String>,
    pub allowed_paths: String,
    pub shell_config: Option<String>,
    pub schedule_kind: String,
    pub schedule_config: String,
    pub continuous_context: bool,
    pub current_workflow_session_id: Option<String>,
    pub self_review: bool,
    pub enabled: bool,
    pub next_run_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkflowAutomationRunInsert {
    pub id: String,
    pub automation_id: String,
    pub workflow_session_id: Option<String>,
    pub status: String,
    pub scheduled_for: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub error: Option<String>,
}

impl From<&Row<'_>> for WorkflowAutomation {
    fn from(row: &Row<'_>) -> Self {
        Self {
            id: row.get("id").unwrap_or_default(),
            title: row.get("title").unwrap_or_default(),
            prompt: row.get("prompt").ok(),
            prompt_file_path: row.get("prompt_file_path").ok(),
            agent_id: row.get("agent_id").unwrap_or_default(),
            agent_config: row.get("agent_config").ok(),
            allowed_paths: row
                .get("allowed_paths")
                .unwrap_or_else(|_| "[]".to_string()),
            shell_config: row.get("shell_config").ok(),
            schedule_kind: row.get("schedule_kind").unwrap_or_default(),
            schedule_config: row
                .get("schedule_config")
                .unwrap_or_else(|_| "{}".to_string()),
            continuous_context: row.get::<_, i64>("continuous_context").unwrap_or(0) != 0,
            current_workflow_session_id: row.get("current_workflow_session_id").ok(),
            self_review: row.get::<_, i64>("self_review").unwrap_or(0) != 0,
            enabled: row.get::<_, i64>("enabled").unwrap_or(1) != 0,
            next_run_at: row.get("next_run_at").ok(),
            last_run_at: row.get("last_run_at").ok(),
            created_at: row.get("created_at").ok(),
            updated_at: row.get("updated_at").ok(),
        }
    }
}

impl From<&Row<'_>> for WorkflowAutomationRun {
    fn from(row: &Row<'_>) -> Self {
        Self {
            id: row.get("id").unwrap_or_default(),
            automation_id: row.get("automation_id").unwrap_or_default(),
            workflow_session_id: row.get("workflow_session_id").ok(),
            status: row.get("status").unwrap_or_default(),
            scheduled_for: row.get("scheduled_for").unwrap_or_default(),
            started_at: row.get("started_at").ok(),
            finished_at: row.get("finished_at").ok(),
            error: row.get("error").ok(),
            created_at: row.get("created_at").ok(),
            updated_at: row.get("updated_at").ok(),
        }
    }
}

impl MainStore {
    fn delete_workflow_tree_tx(tx: &Transaction<'_>, id: &str) -> Result<(), StoreError> {
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

        for workflow_id in &workflow_ids {
            tx.execute("DELETE FROM workflows WHERE id = ?1", params![workflow_id])?;
        }

        Ok(())
    }

    pub fn list_workflow_automations(&self) -> Result<Vec<WorkflowAutomation>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT * FROM workflow_automations
             ORDER BY updated_at DESC, created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| Ok(WorkflowAutomation::from(row)))?;
        let mut automations = Vec::new();
        for row in rows {
            automations.push(row?);
        }
        Ok(automations)
    }

    pub fn get_workflow_automation(
        &self,
        id: &str,
    ) -> Result<Option<WorkflowAutomation>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let mut stmt = conn.prepare("SELECT * FROM workflow_automations WHERE id = ?1")?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(WorkflowAutomation::from(row)))
        } else {
            Ok(None)
        }
    }

    pub fn upsert_workflow_automation(
        &self,
        automation: &WorkflowAutomationUpsert,
    ) -> Result<WorkflowAutomation, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "INSERT INTO workflow_automations
             (id, title, prompt, prompt_file_path, agent_id, agent_config, allowed_paths,
              shell_config, schedule_kind, schedule_config, continuous_context,
              current_workflow_session_id, self_review, enabled, next_run_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(id) DO UPDATE SET
                title = excluded.title,
                prompt = excluded.prompt,
                prompt_file_path = excluded.prompt_file_path,
                agent_id = excluded.agent_id,
                agent_config = excluded.agent_config,
                allowed_paths = excluded.allowed_paths,
                shell_config = excluded.shell_config,
                schedule_kind = excluded.schedule_kind,
                schedule_config = excluded.schedule_config,
                continuous_context = excluded.continuous_context,
                current_workflow_session_id = excluded.current_workflow_session_id,
                self_review = excluded.self_review,
                enabled = excluded.enabled,
                next_run_at = excluded.next_run_at,
                updated_at = CURRENT_TIMESTAMP",
            params![
                automation.id,
                automation.title,
                automation.prompt,
                automation.prompt_file_path,
                automation.agent_id,
                automation.agent_config,
                automation.allowed_paths,
                automation.shell_config,
                automation.schedule_kind,
                automation.schedule_config,
                automation.continuous_context as i64,
                automation.current_workflow_session_id,
                automation.self_review as i64,
                automation.enabled as i64,
                automation.next_run_at,
            ],
        )?;

        conn.query_row(
            "SELECT * FROM workflow_automations WHERE id = ?1",
            params![automation.id],
            |row| Ok(WorkflowAutomation::from(row)),
        )
        .map_err(StoreError::from)
    }

    pub fn delete_workflow_automation(&self, id: &str) -> Result<(), StoreError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let tx = conn.transaction()?;
        let mut workflow_ids = {
            let mut stmt = tx.prepare(
                "SELECT DISTINCT workflow_session_id
                 FROM workflow_automation_runs
                 WHERE automation_id = ?1 AND workflow_session_id IS NOT NULL",
            )?;
            let rows = stmt.query_map(params![id], |row| row.get::<_, String>(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            ids
        };
        if let Some(current_workflow_session_id) = tx
            .query_row(
                "SELECT current_workflow_session_id
                 FROM workflow_automations
                 WHERE id = ?1",
                params![id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten()
        {
            workflow_ids.push(current_workflow_session_id);
        }

        let mut seen_workflow_ids = HashSet::new();
        workflow_ids.retain(|workflow_id| seen_workflow_ids.insert(workflow_id.clone()));

        tx.execute(
            "DELETE FROM workflow_automation_runs WHERE automation_id = ?1",
            params![id],
        )?;

        for workflow_id in &workflow_ids {
            Self::delete_workflow_tree_tx(&tx, workflow_id)?;
        }

        tx.execute(
            "DELETE FROM workflow_automations WHERE id = ?1",
            params![id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_workflow_automation_enabled(
        &self,
        id: &str,
        enabled: bool,
        next_run_at: Option<String>,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflow_automations
             SET enabled = ?2, next_run_at = ?3, updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![id, enabled as i64, next_run_at],
        )?;
        Ok(())
    }

    pub fn update_workflow_automation_run_after_start(
        &self,
        automation_id: &str,
        run_id: &str,
        workflow_session_id: &str,
        scheduled_for: &str,
        current_workflow_session_id: Option<&str>,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflow_automations
             SET last_run_at = ?2, current_workflow_session_id = ?3, updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![automation_id, scheduled_for, current_workflow_session_id],
        )?;
        conn.execute(
            "UPDATE workflow_automation_runs
             SET workflow_session_id = ?2, status = 'running', started_at = CURRENT_TIMESTAMP,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![run_id, workflow_session_id],
        )?;
        Ok(())
    }

    pub fn update_workflow_automation_run_failed(
        &self,
        run_id: &str,
        error: &str,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "UPDATE workflow_automation_runs
             SET status = 'failed', finished_at = CURRENT_TIMESTAMP, error = ?2,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            params![run_id, error],
        )?;
        Ok(())
    }

    pub fn add_workflow_automation_run(
        &self,
        run: &WorkflowAutomationRunInsert,
    ) -> Result<WorkflowAutomationRun, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "INSERT INTO workflow_automation_runs
             (id, automation_id, workflow_session_id, status, scheduled_for,
              started_at, finished_at, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                run.id,
                run.automation_id,
                run.workflow_session_id,
                run.status,
                run.scheduled_for,
                run.started_at,
                run.finished_at,
                run.error,
            ],
        )?;

        conn.query_row(
            "SELECT * FROM workflow_automation_runs WHERE id = ?1",
            params![run.id],
            |row| Ok(WorkflowAutomationRun::from(row)),
        )
        .map_err(StoreError::from)
    }

    pub fn list_workflow_automation_runs(
        &self,
        automation_id: &str,
    ) -> Result<Vec<WorkflowAutomationRun>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT * FROM workflow_automation_runs
             WHERE automation_id = ?1
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![automation_id], |row| {
            Ok(WorkflowAutomationRun::from(row))
        })?;
        let mut runs = Vec::new();
        for row in rows {
            runs.push(row?);
        }
        Ok(runs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::workflow::WorkflowMessage;
    use crate::db::MainStore;
    use crate::db::StoreError;
    use tempfile::tempdir;

    fn create_test_store() -> MainStore {
        let dir = tempdir().expect("failed to create temp dir");
        let db_path = dir.path().join("automation_test.db");
        MainStore::new(db_path).expect("failed to create MainStore")
    }

    fn seed_agent(store: &MainStore, id: &str) {
        let conn = store
            .conn
            .lock()
            .expect("failed to lock db connection for agent seed");
        conn.execute(
            "INSERT INTO agents (id, name, system_prompt, agent_type, max_contexts)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, format!("Agent {}", id), "You are a test agent.", "autonomous", 20],
        )
        .expect("failed to seed agent");
    }

    #[test]
    fn test_delete_workflow_automation_removes_associated_workflows() -> Result<(), StoreError> {
        let store = create_test_store();
        seed_agent(&store, "agent-main");
        seed_agent(&store, "agent-child");

        store.create_workflow("auto-session", "Auto task", "agent-main", None, None)?;
        store.create_workflow(
            "auto-child-session",
            "Auto child task",
            "agent-child",
            None,
            Some("auto-session"),
        )?;

        store.add_workflow_message(&WorkflowMessage {
            id: None,
            session_id: "auto-session".to_string(),
            role: "assistant".to_string(),
            message: "hello".to_string(),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: None,
            metadata: None,
            attached_context: None,
            step_type: None,
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        })?;

        let automation = WorkflowAutomationUpsert {
            id: "automation-1".to_string(),
            title: "Automation".to_string(),
            prompt: Some("run".to_string()),
            prompt_file_path: None,
            agent_id: "agent-main".to_string(),
            agent_config: Some("{}".to_string()),
            allowed_paths: "[]".to_string(),
            shell_config: None,
            schedule_kind: "daily".to_string(),
            schedule_config: "{}".to_string(),
            continuous_context: true,
            current_workflow_session_id: Some("auto-session".to_string()),
            self_review: false,
            enabled: true,
            next_run_at: None,
        };
        store.upsert_workflow_automation(&automation)?;
        store.add_workflow_automation_run(&WorkflowAutomationRunInsert {
            id: "run-1".to_string(),
            automation_id: "automation-1".to_string(),
            workflow_session_id: None,
            status: "completed".to_string(),
            scheduled_for: "2026-06-27 10:00:00".to_string(),
            started_at: None,
            finished_at: None,
            error: None,
        })?;

        store.delete_workflow_automation("automation-1")?;

        assert!(store.get_workflow("auto-session")?.is_none());
        assert!(store.get_workflow("auto-child-session")?.is_none());
        assert!(store.get_workflow_automation("automation-1")?.is_none());
        assert!(store.list_workflow_automation_runs("automation-1")?.is_empty());

        Ok(())
    }
}
