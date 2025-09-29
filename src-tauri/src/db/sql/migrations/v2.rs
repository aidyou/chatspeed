use crate::db::StoreError;
use rusqlite::Connection;

/// Version 2 migration SQL statements
pub const MIGRATION_SQL: &[(&str, &str)] = &[
    // Agents table for ReAct agent configuration
    (
        "agents",
        "CREATE TABLE IF NOT EXISTS agents (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            description TEXT,
            system_prompt TEXT NOT NULL,
            agent_type TEXT NOT NULL DEFAULT 'autonomous', -- 'autonomous' or 'planning'
            planning_prompt TEXT,            -- Prompt for the planning phase
            available_tools TEXT,      -- JSON array of available tool IDs
            auto_approve TEXT,         -- JSON array of tools that can be executed without user confirmation
            plan_model TEXT,           -- Model for planning phase
            act_model TEXT,            -- Model for action phase
            vision_model TEXT,         -- Vision model (reserved)
            max_contexts INTEGER DEFAULT 128000,  -- Maximum context length (in tokens)
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )"
    ),
];

/// Applies version 2 database migration
pub fn run_migration(conn: &mut Connection) -> Result<(), StoreError> {
    // It's a migration example for new version
    if MIGRATION_SQL.is_empty() {
        return Ok(());
    }

    // start transaction
    let tx = conn.transaction()?;

    // execute all migration SQL
    for (_name, sql) in MIGRATION_SQL {
        tx.execute(sql, [])?;
    }

    // insert database version
    tx.execute("INSERT INTO db_version (version) VALUES (2)", [])?;

    // commit transaction
    tx.commit()?;

    Ok(())
}
