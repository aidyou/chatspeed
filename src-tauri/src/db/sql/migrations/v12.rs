use super::common::{column_exists, MigrationDefinition};
use crate::db::StoreError;
use rusqlite::Connection;

pub const MIGRATION_SQL: &[(&str, &str)] = &[];

fn ensure_workflow_usage_attribution(conn: &Connection) -> Result<(), StoreError> {
    let columns = [
        ("workflow_session_id", "TEXT"),
        ("workflow_task_run_id", "TEXT"),
        ("workflow_segment_id", "INTEGER"),
        ("root_session_id", "TEXT"),
        ("root_task_run_id", "TEXT"),
        ("request_kind", "TEXT"),
    ];

    for (name, sql_type) in columns {
        if !column_exists(conn, "ccproxy_stats", name)? {
            conn.execute(
                &format!("ALTER TABLE ccproxy_stats ADD COLUMN {name} {sql_type}"),
                [],
            )?;
        }
    }

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_ccproxy_stats_workflow_task_run
         ON ccproxy_stats(workflow_session_id, workflow_task_run_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_ccproxy_stats_root_task_run
         ON ccproxy_stats(root_session_id, root_task_run_id)",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS workflow_task_usage (
            session_id TEXT NOT NULL,
            task_run_id TEXT NOT NULL,
            root_session_id TEXT NOT NULL,
            root_task_run_id TEXT NOT NULL,
            terminal_status TEXT NOT NULL,
            started_at TEXT,
            ended_at TEXT,
            duration_ms INTEGER,
            summary_json TEXT NOT NULL,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            PRIMARY KEY (session_id, task_run_id)
        )",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_workflow_task_usage_root_task_run
         ON workflow_task_usage(root_session_id, root_task_run_id)",
        [],
    )?;

    Ok(())
}

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 12,
    description: "v12 migration: Add workflow usage attribution to ccproxy stats",
    sql: MIGRATION_SQL,
    ensure: Some(ensure_workflow_usage_attribution),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_attribution_columns_and_indexes_idempotently() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        conn.execute_batch(
            "CREATE TABLE ccproxy_stats (
                client_model TEXT NOT NULL,
                backend_model TEXT NOT NULL,
                provider TEXT NOT NULL,
                protocol TEXT NOT NULL,
                tool_compat_mode INTEGER,
                status_code INTEGER NOT NULL,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cache_tokens INTEGER
            );",
        )
        .expect("failed to create migration fixture");

        ensure_workflow_usage_attribution(&conn).expect("v12 migration should succeed");
        ensure_workflow_usage_attribution(&conn).expect("v12 migration should be idempotent");

        assert!(column_exists(&conn, "ccproxy_stats", "workflow_task_run_id").unwrap());
        assert!(column_exists(&conn, "ccproxy_stats", "root_task_run_id").unwrap());
        assert!(column_exists(&conn, "ccproxy_stats", "request_kind").unwrap());
    }
}
