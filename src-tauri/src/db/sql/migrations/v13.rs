use super::common::{column_exists, MigrationDefinition};
use crate::db::StoreError;
use rusqlite::Connection;

pub const MIGRATION_SQL: &[(&str, &str)] = &[(
    "sandbox_schemes",
    "CREATE TABLE IF NOT EXISTS sandbox_schemes (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL UNIQUE,
        description TEXT NOT NULL DEFAULT '',
        config TEXT NOT NULL,
        disabled INTEGER NOT NULL DEFAULT 0,
        created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
        updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
    )",
)];

fn ensure_shared_sandbox_schemes(conn: &Connection) -> Result<(), StoreError> {
    for (_, sql) in MIGRATION_SQL {
        conn.execute(sql, [])?;
    }

    if !column_exists(conn, "agents", "sandbox_execution_mode")? {
        conn.execute(
            "ALTER TABLE agents ADD COLUMN sandbox_execution_mode TEXT NOT NULL DEFAULT 'host_only'",
            [],
        )?;
    }
    if !column_exists(conn, "agents", "sandbox_scheme_id")? {
        conn.execute("ALTER TABLE agents ADD COLUMN sandbox_scheme_id TEXT", [])?;
    }

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agents_sandbox_scheme_id
         ON agents(sandbox_scheme_id)",
        [],
    )?;
    Ok(())
}

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 13,
    description: "v13 migration: Add shared shell sandbox schemes",
    sql: MIGRATION_SQL,
    ensure: Some(ensure_shared_sandbox_schemes),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_shared_sandbox_schema_idempotently() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        conn.execute_batch(
            "CREATE TABLE agents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL
            );",
        )
        .expect("failed to create migration fixture");

        ensure_shared_sandbox_schemes(&conn).expect("v13 migration should succeed");
        ensure_shared_sandbox_schemes(&conn).expect("v13 migration should be idempotent");

        assert!(column_exists(&conn, "agents", "sandbox_execution_mode").unwrap());
        assert!(column_exists(&conn, "agents", "sandbox_scheme_id").unwrap());
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM sqlite_master WHERE type = 'table' AND name = 'sandbox_schemes'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}
