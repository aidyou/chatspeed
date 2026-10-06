use super::common::MigrationDefinition;
use crate::db::StoreError;
use rusqlite::Connection;

pub const MIGRATION_SQL: &[(&str, &str)] = &[];

fn ensure_pending_sub_agent_approval_index(conn: &Connection) -> Result<(), StoreError> {
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_workflows_child_approval_status
         ON workflows(status, parent_session_id, created_at)",
        [],
    )?;
    Ok(())
}

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 16,
    description: "v16 migration: Index child workflows awaiting approval",
    sql: MIGRATION_SQL,
    ensure: Some(ensure_pending_sub_agent_approval_index),
    apply: None,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_pending_sub_agent_approval_index_idempotently() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        conn.execute_batch(
            "CREATE TABLE workflows (
                id TEXT PRIMARY KEY,
                parent_session_id TEXT,
                status TEXT,
                created_at DATETIME
            );",
        )
        .expect("failed to create migration fixture");

        ensure_pending_sub_agent_approval_index(&conn)
            .expect("pending-approval index creation should succeed");
        ensure_pending_sub_agent_approval_index(&conn)
            .expect("pending-approval index creation should be idempotent");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM sqlite_master
                 WHERE type = 'index' AND name = 'idx_workflows_child_approval_status'",
                [],
                |row| row.get(0),
            )
            .expect("failed to query pending-approval index");
        assert_eq!(count, 1);
    }
}
