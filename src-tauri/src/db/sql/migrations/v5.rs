use super::{column_exists, MigrationDefinition};
use crate::db::StoreError;
use rusqlite::Connection;

/// Version 5 migration SQL statements
pub const MIGRATION_SQL: &[(&str, &str)] = &[
    // Workflows table
    (
        "workflows",
        "CREATE TABLE IF NOT EXISTS workflows (
            id TEXT PRIMARY KEY,
            parent_session_id TEXT REFERENCES workflows(id),
            title TEXT,
            user_query TEXT NOT NULL,
            todo_list TEXT,
            status TEXT DEFAULT 'pending',
            agent_id TEXT REFERENCES agents(id),
            agent_config TEXT,                 -- Unified JSON config (models, shell_policy, etc.)
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )"
    ),
    (
        "idx_workflows_updated_at",
        "CREATE INDEX IF NOT EXISTS idx_workflows_updated_at ON workflows(updated_at DESC)"
    ),
    (
        "idx_workflows_parent_session_id",
        "CREATE INDEX IF NOT EXISTS idx_workflows_parent_session_id ON workflows(parent_session_id)"
    ),
    // Workflow messages table
    (
        "workflow_messages",
        "CREATE TABLE IF NOT EXISTS workflow_messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            role TEXT NOT NULL,
            message TEXT NOT NULL,
            reasoning TEXT,
            message_kind TEXT NOT NULL DEFAULT 'message',
            message_subtype TEXT,
            segment_id INTEGER NOT NULL DEFAULT 1,
            source_event_type TEXT,
            metadata TEXT,
            attached_context TEXT,             -- Hidden content for AI context only
            step_type TEXT,                    -- Enum: 'think', 'act', 'observe'
            step_index INTEGER DEFAULT 0,      -- The index of the step in the current session
            is_error INTEGER DEFAULT 0,        -- 0 for false, 1 for true
            error_type TEXT,                   -- Enum: 'Security', 'Io', 'InvalidParams', 'Network', 'Auth', 'Other'
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (session_id) REFERENCES workflows(id)
        )"
    ),
    (
        "idx_workflow_messages_session_id",
        "CREATE INDEX IF NOT EXISTS idx_workflow_messages_session_id ON workflow_messages(session_id)"
    ),
    // Workflow context projection table for AI-only prompt history
    (
        "workflow_context_messages",
        "CREATE TABLE IF NOT EXISTS workflow_context_messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            segment_id INTEGER NOT NULL,
            role TEXT NOT NULL,
            message TEXT NOT NULL,
            reasoning TEXT,
            message_kind TEXT NOT NULL DEFAULT 'message',
            message_subtype TEXT,
            metadata TEXT,
            source_message_id INTEGER,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (session_id) REFERENCES workflows(id),
            FOREIGN KEY (source_message_id) REFERENCES workflow_messages(id)
        )"
    ),
    (
        "idx_workflow_context_messages_session_segment_id",
        "CREATE INDEX IF NOT EXISTS idx_workflow_context_messages_session_segment_id
         ON workflow_context_messages(session_id, segment_id, id)"
    ),
    // Workflow snapshots table for ExecutionContext recovery
    (
        "workflow_snapshots",
        "CREATE TABLE IF NOT EXISTS workflow_snapshots (
            session_id TEXT PRIMARY KEY,
            context_json TEXT NOT NULL,
            version TEXT NOT NULL,
            state TEXT,
            wait_reason TEXT,
            waiting_on_sub_agent_id TEXT,
            sub_agent_sessions TEXT DEFAULT '[]',
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )"
    ),
    (
        "idx_workflow_snapshots_updated_at",
        "CREATE INDEX IF NOT EXISTS idx_workflow_snapshots_updated_at ON workflow_snapshots(updated_at DESC)"
    ),
    (
        "idx_workflow_snapshots_waiting_on_sub_agent_id",
        "CREATE INDEX IF NOT EXISTS idx_workflow_snapshots_waiting_on_sub_agent_id ON workflow_snapshots(waiting_on_sub_agent_id)"
    ),
    // Workflow events table for structured event auditing
    (
        "workflow_events",
        "CREATE TABLE IF NOT EXISTS workflow_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            event_type TEXT NOT NULL,
            event_version TEXT NOT NULL,
            event_data TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )"
    ),
    (
        "idx_workflow_events_session_id_id",
        "CREATE INDEX IF NOT EXISTS idx_workflow_events_session_id_id ON workflow_events(session_id, id)"
    ),
];

fn ensure_agent_hierarchy_columns(conn: &Connection) -> Result<(), StoreError> {
    if !column_exists(conn, "agents", "models")? {
        conn.execute("ALTER TABLE agents ADD COLUMN models TEXT", [])?;
    }

    if !column_exists(conn, "agents", "shell_policy")? {
        conn.execute("ALTER TABLE agents ADD COLUMN shell_policy TEXT", [])?;
    }

    if !column_exists(conn, "agents", "allowed_paths")? {
        conn.execute("ALTER TABLE agents ADD COLUMN allowed_paths TEXT", [])?;
    }

    if !column_exists(conn, "agents", "final_audit")? {
        conn.execute(
            "ALTER TABLE agents ADD COLUMN final_audit BOOLEAN DEFAULT 0",
            [],
        )?;
    }

    if !column_exists(conn, "agents", "approval_level")? {
        conn.execute(
            "ALTER TABLE agents ADD COLUMN approval_level TEXT DEFAULT 'default'",
            [],
        )?;
    }

    if !column_exists(conn, "agents", "role")? {
        conn.execute(
            "ALTER TABLE agents ADD COLUMN role TEXT DEFAULT 'primary'",
            [],
        )?;
    }

    if !column_exists(conn, "agents", "parent_agent_id")? {
        conn.execute(
            "ALTER TABLE agents ADD COLUMN parent_agent_id TEXT REFERENCES agents(id)",
            [],
        )?;
    }

    if !column_exists(conn, "agents", "skill_enabled")? {
        conn.execute(
            "ALTER TABLE agents ADD COLUMN skill_enabled BOOLEAN DEFAULT 1",
            [],
        )?;
    }

    if !column_exists(conn, "agents", "is_system")? {
        conn.execute(
            "ALTER TABLE agents ADD COLUMN is_system BOOLEAN NOT NULL DEFAULT 0",
            [],
        )?;
    }

    if !column_exists(conn, "agents", "disabled")? {
        conn.execute(
            "ALTER TABLE agents ADD COLUMN disabled BOOLEAN NOT NULL DEFAULT 0",
            [],
        )?;
    }

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agents_parent_agent_id ON agents(parent_agent_id)",
        [],
    )?;

    Ok(())
}

fn ensure_workflow_parent_column(conn: &Connection) -> Result<(), StoreError> {
    if !column_exists(conn, "workflows", "parent_session_id")? {
        conn.execute(
            "ALTER TABLE workflows ADD COLUMN parent_session_id TEXT REFERENCES workflows(id)",
            [],
        )?;
    }

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_workflows_parent_session_id ON workflows(parent_session_id)",
        [],
    )?;

    Ok(())
}

fn ensure_workflow_message_columns(conn: &Connection) -> Result<(), StoreError> {
    if !column_exists(conn, "workflow_messages", "message_kind")? {
        conn.execute(
            "ALTER TABLE workflow_messages ADD COLUMN message_kind TEXT NOT NULL DEFAULT 'message'",
            [],
        )?;
    }

    if !column_exists(conn, "workflow_messages", "message_subtype")? {
        conn.execute(
            "ALTER TABLE workflow_messages ADD COLUMN message_subtype TEXT",
            [],
        )?;
    }

    if !column_exists(conn, "workflow_messages", "segment_id")? {
        conn.execute(
            "ALTER TABLE workflow_messages ADD COLUMN segment_id INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }

    if !column_exists(conn, "workflow_messages", "source_event_type")? {
        conn.execute(
            "ALTER TABLE workflow_messages ADD COLUMN source_event_type TEXT",
            [],
        )?;
    }

    Ok(())
}

pub fn ensure(conn: &Connection) -> Result<(), StoreError> {
    ensure_agent_hierarchy_columns(conn)?;
    ensure_workflow_parent_column(conn)?;
    ensure_workflow_message_columns(conn)?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS workflow_context_messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            segment_id INTEGER NOT NULL,
            role TEXT NOT NULL,
            message TEXT NOT NULL,
            reasoning TEXT,
            message_kind TEXT NOT NULL DEFAULT 'message',
            message_subtype TEXT,
            metadata TEXT,
            source_message_id INTEGER,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (session_id) REFERENCES workflows(id),
            FOREIGN KEY (source_message_id) REFERENCES workflow_messages(id)
        )",
        [],
    )?;
    if !column_exists(conn, "workflow_context_messages", "message_kind")? {
        conn.execute(
            "ALTER TABLE workflow_context_messages ADD COLUMN message_kind TEXT NOT NULL DEFAULT 'message'",
            [],
        )?;
    }
    if !column_exists(conn, "workflow_context_messages", "message_subtype")? {
        conn.execute(
            "ALTER TABLE workflow_context_messages ADD COLUMN message_subtype TEXT",
            [],
        )?;
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_workflow_context_messages_session_segment_id
         ON workflow_context_messages(session_id, segment_id, id)",
        [],
    )?;
    Ok(())
}

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 5,
    description: "v5 migration: Add workflows table and unified agent configuration columns",
    sql: MIGRATION_SQL,
    ensure: Some(ensure),
};
