use super::common::{column_exists, MigrationDefinition};
use crate::db::StoreError;
use rusqlite::Connection;

pub const MIGRATION_SQL: &[(&str, &str)] = &[];

fn ensure_sub_agent_roles(conn: &Connection) -> Result<(), StoreError> {
    if !column_exists(conn, "agents", "sub_agent_role")? {
        conn.execute("ALTER TABLE agents ADD COLUMN sub_agent_role TEXT", [])?;
    }

    conn.execute(
        "UPDATE agents
         SET sub_agent_role = NULL
         WHERE COALESCE(role, agent_type, 'primary') != 'child'",
        [],
    )?;
    conn.execute(
        "UPDATE agents SET sub_agent_role = 'explorer' WHERE id = 'builtin:code-explorer'",
        [],
    )?;
    conn.execute(
        "UPDATE agents SET sub_agent_role = 'final_reviewer' WHERE id = 'builtin:final-code-reviewer'",
        [],
    )?;
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_agents_parent_sub_agent_role
         ON agents(parent_agent_id, sub_agent_role)
         WHERE parent_agent_id IS NOT NULL AND sub_agent_role IS NOT NULL",
        [],
    )?;

    Ok(())
}

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 11,
    description: "v11 migration: Add parent-scoped sub-agent responsibility roles",
    sql: MIGRATION_SQL,
    ensure: Some(ensure_sub_agent_roles),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backfills_builtin_roles_and_enforces_parent_scoped_uniqueness() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        conn.execute_batch(
            "CREATE TABLE agents (
                id TEXT PRIMARY KEY,
                role TEXT,
                agent_type TEXT,
                parent_agent_id TEXT
            );
            INSERT INTO agents (id, role, parent_agent_id)
            VALUES ('builtin:code-explorer', 'child', 'builtin:coding');
            INSERT INTO agents (id, role, parent_agent_id)
            VALUES ('builtin:final-code-reviewer', 'child', 'builtin:coding');",
        )
        .expect("failed to create migration fixture");

        ensure_sub_agent_roles(&conn).expect("v11 migration should succeed");

        let explorer_role: String = conn
            .query_row(
                "SELECT sub_agent_role FROM agents WHERE id = 'builtin:code-explorer'",
                [],
                |row| row.get(0),
            )
            .expect("failed to read explorer role");
        let reviewer_role: String = conn
            .query_row(
                "SELECT sub_agent_role FROM agents WHERE id = 'builtin:final-code-reviewer'",
                [],
                |row| row.get(0),
            )
            .expect("failed to read reviewer role");
        assert_eq!(explorer_role, "explorer");
        assert_eq!(reviewer_role, "final_reviewer");

        conn.execute(
            "INSERT INTO agents (id, role, parent_agent_id, sub_agent_role)
             VALUES ('other-parent-reviewer', 'child', 'other-parent', 'final_reviewer')",
            [],
        )
        .expect("different parents may each own a final reviewer");
        assert!(conn
            .execute(
                "INSERT INTO agents (id, role, parent_agent_id, sub_agent_role)
                 VALUES ('duplicate-reviewer', 'child', 'builtin:coding', 'final_reviewer')",
                [],
            )
            .is_err());
    }
}
