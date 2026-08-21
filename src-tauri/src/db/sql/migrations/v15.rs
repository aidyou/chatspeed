use super::common::{column_exists, MigrationDefinition};
use crate::db::StoreError;
use rusqlite::Connection;

pub const MIGRATION_SQL: &[(&str, &str)] = &[];

fn ensure_agent_personality_column(conn: &Connection) -> Result<(), StoreError> {
    if !column_exists(conn, "agents", "personality")? {
        conn.execute("ALTER TABLE agents ADD COLUMN personality TEXT", [])?;
    }
    Ok(())
}

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 15,
    description: "v15 migration: Add optional Agent personality configuration",
    sql: MIGRATION_SQL,
    ensure: Some(ensure_agent_personality_column),
    apply: None,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_agent_personality_column_idempotently() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        conn.execute_batch(
            "CREATE TABLE agents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL
            );",
        )
        .expect("failed to create migration fixture");

        ensure_agent_personality_column(&conn).expect("v15 migration should succeed");
        ensure_agent_personality_column(&conn).expect("v15 migration should be idempotent");

        assert!(column_exists(&conn, "agents", "personality").unwrap());
    }
}
