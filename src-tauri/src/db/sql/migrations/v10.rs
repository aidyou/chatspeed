use super::common::{column_exists, MigrationDefinition};
use crate::db::StoreError;
use rusqlite::{params, Connection};

const OLD_COMPLETION_TOOL_NAME: &str = "complete_workflow_with_summary";
const COMPLETION_TOOL_NAME: &str = "complete_workflow";

pub const MIGRATION_SQL: &[(&str, &str)] = &[];

fn ensure_mcp_tool_exposure_column(conn: &Connection) -> Result<(), StoreError> {
    if !column_exists(conn, "agents", "mcp_tool_exposure")? {
        conn.execute("ALTER TABLE agents ADD COLUMN mcp_tool_exposure TEXT", [])?;
    }
    Ok(())
}

fn migrate_completion_tool_name(conn: &Connection) -> Result<(), StoreError> {
    let contains_old_name = format!("%{OLD_COMPLETION_TOOL_NAME}%");

    conn.execute(
        "UPDATE agents
         SET system_prompt = REPLACE(system_prompt, ?1, ?2),
             planning_prompt = REPLACE(planning_prompt, ?1, ?2),
             available_tools = REPLACE(available_tools, ?1, ?2),
             auto_approve = REPLACE(auto_approve, ?1, ?2)
         WHERE system_prompt LIKE ?3
            OR planning_prompt LIKE ?3
            OR available_tools LIKE ?3
            OR auto_approve LIKE ?3",
        params![
            OLD_COMPLETION_TOOL_NAME,
            COMPLETION_TOOL_NAME,
            contains_old_name
        ],
    )?;
    conn.execute(
        "UPDATE workflows
         SET agent_config = REPLACE(agent_config, ?1, ?2)
         WHERE agent_config LIKE ?3",
        params![
            OLD_COMPLETION_TOOL_NAME,
            COMPLETION_TOOL_NAME,
            contains_old_name
        ],
    )?;
    conn.execute(
        "UPDATE workflow_automations
         SET agent_config = REPLACE(agent_config, ?1, ?2)
         WHERE agent_config LIKE ?3",
        params![
            OLD_COMPLETION_TOOL_NAME,
            COMPLETION_TOOL_NAME,
            contains_old_name
        ],
    )?;
    conn.execute(
        "UPDATE workflow_messages
         SET metadata = REPLACE(metadata, ?1, ?2),
             attached_context = REPLACE(attached_context, ?1, ?2)
         WHERE metadata LIKE ?3
            OR attached_context LIKE ?3",
        params![
            OLD_COMPLETION_TOOL_NAME,
            COMPLETION_TOOL_NAME,
            contains_old_name
        ],
    )?;
    conn.execute(
        "UPDATE workflow_context_messages
         SET metadata = REPLACE(metadata, ?1, ?2)
         WHERE metadata LIKE ?3",
        params![
            OLD_COMPLETION_TOOL_NAME,
            COMPLETION_TOOL_NAME,
            contains_old_name
        ],
    )?;
    conn.execute(
        "UPDATE workflow_snapshots
         SET context_json = REPLACE(context_json, ?1, ?2)
         WHERE context_json LIKE ?3",
        params![
            OLD_COMPLETION_TOOL_NAME,
            COMPLETION_TOOL_NAME,
            contains_old_name
        ],
    )?;
    conn.execute(
        "UPDATE workflow_events
         SET event_data = REPLACE(event_data, ?1, ?2)
         WHERE event_data LIKE ?3",
        params![
            OLD_COMPLETION_TOOL_NAME,
            COMPLETION_TOOL_NAME,
            contains_old_name
        ],
    )?;

    Ok(())
}

fn ensure_v10_data(conn: &Connection) -> Result<(), StoreError> {
    ensure_mcp_tool_exposure_column(conn)?;
    migrate_completion_tool_name(conn)
}

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 10,
    description:
        "v10 migration: Persist per-agent direct MCP tool exposure and rename completion tool",
    sql: MIGRATION_SQL,
    ensure: Some(ensure_v10_data),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_persisted_completion_tool_references() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        conn.execute_batch(
            "CREATE TABLE agents (
                system_prompt TEXT,
                planning_prompt TEXT,
                available_tools TEXT,
                auto_approve TEXT,
                mcp_tool_exposure TEXT
            );
            CREATE TABLE workflows (agent_config TEXT);
            CREATE TABLE workflow_automations (agent_config TEXT);
            CREATE TABLE workflow_messages (metadata TEXT, attached_context TEXT);
            CREATE TABLE workflow_context_messages (metadata TEXT);
            CREATE TABLE workflow_snapshots (context_json TEXT);
            CREATE TABLE workflow_events (event_data TEXT);",
        )
        .expect("failed to create migration test tables");

        let old = OLD_COMPLETION_TOOL_NAME;
        conn.execute(
            "INSERT INTO agents (system_prompt, planning_prompt, available_tools, auto_approve)
             VALUES (?1, ?1, ?1, ?1)",
            [old],
        )
        .expect("failed to seed agent");
        for statement in [
            "INSERT INTO workflows (agent_config) VALUES (?1)",
            "INSERT INTO workflow_automations (agent_config) VALUES (?1)",
            "INSERT INTO workflow_messages (metadata, attached_context) VALUES (?1, ?1)",
            "INSERT INTO workflow_context_messages (metadata) VALUES (?1)",
            "INSERT INTO workflow_snapshots (context_json) VALUES (?1)",
            "INSERT INTO workflow_events (event_data) VALUES (?1)",
        ] {
            conn.execute(statement, [old])
                .expect("failed to seed workflow data");
        }

        ensure_v10_data(&conn).expect("v10 data migration should succeed");

        for query in [
            "SELECT system_prompt FROM agents",
            "SELECT planning_prompt FROM agents",
            "SELECT available_tools FROM agents",
            "SELECT auto_approve FROM agents",
            "SELECT agent_config FROM workflows",
            "SELECT agent_config FROM workflow_automations",
            "SELECT metadata FROM workflow_messages",
            "SELECT attached_context FROM workflow_messages",
            "SELECT metadata FROM workflow_context_messages",
            "SELECT context_json FROM workflow_snapshots",
            "SELECT event_data FROM workflow_events",
        ] {
            let value: String = conn
                .query_row(query, [], |row| row.get(0))
                .expect("failed to read migrated value");
            assert_eq!(value, COMPLETION_TOOL_NAME);
        }
    }
}
