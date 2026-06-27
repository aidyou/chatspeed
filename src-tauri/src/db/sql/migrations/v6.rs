use super::{column_exists, MigrationDefinition};
use crate::db::StoreError;
use rusqlite::Connection;
use serde_json::Value;

/// Version 6 migration SQL statements.
/// Adds workflow automation definitions and run history.
pub const MIGRATION_SQL: &[(&str, &str)] = &[
    (
        "workflow_automations",
        "CREATE TABLE IF NOT EXISTS workflow_automations (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            prompt TEXT,
            prompt_file_path TEXT,
            agent_id TEXT NOT NULL REFERENCES agents(id),
            agent_config TEXT,
            allowed_paths TEXT NOT NULL DEFAULT '[]',
            shell_config TEXT,
            schedule_kind TEXT NOT NULL,
            schedule_config TEXT NOT NULL,
            continuous_context INTEGER NOT NULL DEFAULT 0,
            current_workflow_session_id TEXT,
            self_review INTEGER NOT NULL DEFAULT 0,
            enabled INTEGER NOT NULL DEFAULT 1,
            next_run_at DATETIME,
            last_run_at DATETIME,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
    ),
    (
        "idx_workflow_automations_enabled_next_run_at",
        "CREATE INDEX IF NOT EXISTS idx_workflow_automations_enabled_next_run_at
         ON workflow_automations(enabled, next_run_at)",
    ),
    (
        "idx_workflow_automations_updated_at",
        "CREATE INDEX IF NOT EXISTS idx_workflow_automations_updated_at
         ON workflow_automations(updated_at DESC)",
    ),
    (
        "workflow_automation_runs",
        "CREATE TABLE IF NOT EXISTS workflow_automation_runs (
            id TEXT PRIMARY KEY,
            automation_id TEXT NOT NULL REFERENCES workflow_automations(id),
            workflow_session_id TEXT REFERENCES workflows(id),
            status TEXT NOT NULL,
            scheduled_for DATETIME NOT NULL,
            started_at DATETIME,
            finished_at DATETIME,
            error TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
    ),
    (
        "idx_workflow_automation_runs_automation_created_at",
        "CREATE INDEX IF NOT EXISTS idx_workflow_automation_runs_automation_created_at
         ON workflow_automation_runs(automation_id, created_at DESC)",
    ),
    (
        "idx_workflow_automation_runs_workflow_session_id",
        "CREATE INDEX IF NOT EXISTS idx_workflow_automation_runs_workflow_session_id
         ON workflow_automation_runs(workflow_session_id)",
    ),
];

fn ensure_workflow_automation_shell_config(conn: &Connection) -> Result<(), StoreError> {
    if !column_exists(conn, "workflow_automations", "continuous_context")? {
        conn.execute(
            "ALTER TABLE workflow_automations ADD COLUMN continuous_context INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }

    if !column_exists(conn, "workflow_automations", "current_workflow_session_id")? {
        conn.execute(
            "ALTER TABLE workflow_automations ADD COLUMN current_workflow_session_id TEXT",
            [],
        )?;
    }

    if !column_exists(conn, "workflow_automations", "shell_config")? {
        conn.execute(
            "ALTER TABLE workflow_automations ADD COLUMN shell_config TEXT",
            [],
        )?;
    }

    let mut stmt = conn.prepare(
        "SELECT id, schedule_config
         FROM workflow_automations
         WHERE schedule_kind = 'interval'",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    for row in rows {
        let (id, schedule_config) = row?;
        let Ok(mut value) = serde_json::from_str::<Value>(&schedule_config) else {
            continue;
        };

        let Some(object) = value.as_object_mut() else {
            continue;
        };

        if object.contains_key("interval_minutes") {
            continue;
        }

        let interval_hours = object
            .remove("interval_hours")
            .and_then(|value| value.as_u64())
            .unwrap_or(1);
        object.insert(
            "interval_minutes".to_string(),
            Value::from(interval_hours.saturating_mul(60)),
        );

        let updated_schedule_config = serde_json::to_string(&value)?;
        conn.execute(
            "UPDATE workflow_automations
             SET schedule_config = ?2, updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            [&id, &updated_schedule_config],
        )?;
    }

    Ok(())
}

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 6,
    description: "v6 migration: Add workflow automation tables",
    sql: MIGRATION_SQL,
    ensure: Some(ensure_workflow_automation_shell_config),
};
