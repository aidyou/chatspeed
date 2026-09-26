use super::common::{column_exists, MigrationDefinition};
use crate::db::StoreError;
use rusqlite::Connection;

/// Phase 3D local automation concurrency and idempotency schema.
///
/// This migration is additive only. It introduces the durable evidence the
/// `AutomationApplicationService` needs to (1) serialize updates with a
/// compare-and-set revision, (2) guarantee at most one scheduled run per
/// automation slot even across scheduler ticks and restarts, and (3) replay a
/// mutation with the same idempotency key without executing it twice.
///
/// It never touches an existing table row and it never becomes a second
/// workflow/automation state machine: `automation_operations` stores only a
/// redacted mutation envelope and a stable result reference, while the run and
/// workflow snapshot remain the lifecycle authority (INV-6/INV-7/INV-10).
pub const MIGRATION_SQL: &[(&str, &str)] = &[
    (
        "automation_operations",
        "CREATE TABLE IF NOT EXISTS automation_operations (
            operation_id TEXT PRIMARY KEY,
            actor_scope TEXT NOT NULL,
            operation TEXT NOT NULL,
            idempotency_key TEXT NOT NULL,
            request_hash TEXT NOT NULL,
            status TEXT NOT NULL CHECK (status IN ('in_progress','completed','failed')),
            result_json TEXT,
            error_code TEXT,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            UNIQUE (actor_scope, idempotency_key)
        )",
    ),
    (
        "idx_automation_operations_status",
        "CREATE INDEX IF NOT EXISTS idx_automation_operations_status
            ON automation_operations (status, created_at_ms)",
    ),
];

/// Adds the automation revision, run trigger/dispatch columns and the durable
/// scheduled-slot unique index.
///
/// Every statement is guarded so it is a no-op once the schema exists. The
/// columns are added here rather than in `MIGRATION_SQL` because the partial
/// unique index on `workflow_automation_runs.dispatch_key` references a column
/// that only exists after the `ALTER TABLE`, and because a database created by
/// an earlier consolidated-CLI build may already record a version higher than
/// this one; `run_migrations` then skips the SQL step, and without this hook the
/// columns and index would be missing while the facade is running.
fn ensure_automation_concurrency_schema(conn: &Connection) -> Result<(), StoreError> {
    if !column_exists(conn, "workflow_automations", "revision")? {
        conn.execute(
            "ALTER TABLE workflow_automations ADD COLUMN revision INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }

    if !column_exists(conn, "workflow_automation_runs", "trigger")? {
        conn.execute(
            "ALTER TABLE workflow_automation_runs ADD COLUMN trigger TEXT NOT NULL DEFAULT 'manual'",
            [],
        )?;
    }

    if !column_exists(conn, "workflow_automation_runs", "dispatch_key")? {
        conn.execute(
            "ALTER TABLE workflow_automation_runs ADD COLUMN dispatch_key TEXT",
            [],
        )?;
    }

    // Durable scheduled-slot dedupe. NULL dispatch keys (manual/legacy runs)
    // are excluded, so an automation may have any number of manual runs while
    // each scheduled slot can hold at most one row (AC-6/INV-6).
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_automation_runs_dispatch
         ON workflow_automation_runs(automation_id, dispatch_key)
         WHERE dispatch_key IS NOT NULL",
        [],
    )?;

    // Re-apply the receipt table for version-skew databases whose recorded
    // version is already at or past this migration (mirrors v20 robustness).
    for (name, sql) in MIGRATION_SQL {
        conn.execute(sql, [])
            .map_err(|e| StoreError::Query(format!("v21 ensure {name} failed: {e}")))?;
    }

    Ok(())
}

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 21,
    description: "v21 migration: Add automation revision, scheduled dispatch dedupe and mutation receipts",
    sql: MIGRATION_SQL,
    ensure: Some(ensure_automation_concurrency_schema),
    apply: None,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sql::migrations::manager::run_migrations;
    use rusqlite::Connection;

    fn has_column(conn: &Connection, table: &str, column: &str) -> bool {
        column_exists(conn, table, column).expect("pragma table_info should succeed")
    }

    fn index_exists(conn: &Connection, index: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(1) FROM sqlite_master WHERE type = 'index' AND name = ?1",
            [index],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count == 1)
        .expect("index existence query should succeed")
    }

    #[test]
    fn fresh_schema_includes_v21_automation_columns() {
        let mut conn = Connection::open_in_memory().expect("open in-memory sqlite");
        run_migrations(&mut conn).expect("fresh migrations should succeed");

        assert!(has_column(&conn, "workflow_automations", "revision"));
        assert!(has_column(&conn, "workflow_automation_runs", "trigger"));
        assert!(has_column(&conn, "workflow_automation_runs", "dispatch_key"));
        assert!(index_exists(&conn, "idx_workflow_automation_runs_dispatch"));
    }
}
