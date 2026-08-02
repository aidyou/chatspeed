use super::common::{column_exists, MigrationDefinition};
use crate::db::StoreError;
use rusqlite::Connection;

pub const MIGRATION_SQL: &[(&str, &str)] = &[];

fn ensure_agent_sandbox_config(conn: &Connection) -> Result<(), StoreError> {
    if !column_exists(conn, "agents", "sandbox_config")? {
        conn.execute("ALTER TABLE agents ADD COLUMN sandbox_config TEXT", [])?;
    }

    Ok(())
}

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 13,
    description: "v13 migration: Add Agent shell sandbox configuration",
    sql: MIGRATION_SQL,
    ensure: Some(ensure_agent_sandbox_config),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_sandbox_config_column_idempotently() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        conn.execute_batch(
            "CREATE TABLE agents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL
            );",
        )
        .expect("failed to create migration fixture");

        ensure_agent_sandbox_config(&conn).expect("v13 migration should succeed");
        ensure_agent_sandbox_config(&conn).expect("v13 migration should be idempotent");

        assert!(column_exists(&conn, "agents", "sandbox_config").unwrap());
    }
}
