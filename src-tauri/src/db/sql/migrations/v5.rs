/// Version 5 migration SQL statements
pub const MIGRATION_SQL: &[(&str, &str)] = &[
    // Workflows table
    (
        "workflows",
        "CREATE TABLE IF NOT EXISTS workflows (
            id TEXT PRIMARY KEY,
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
    // Workflow messages table
    (
        "workflow_messages",
        "CREATE TABLE IF NOT EXISTS workflow_messages (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            role TEXT NOT NULL,
            message TEXT NOT NULL,
            reasoning TEXT,
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
    // Add unified models JSON column to agents
    (
        "agents_v5_models",
        "ALTER TABLE agents ADD COLUMN models TEXT"
    ),
    // Add shell_policy JSON column to agents
    (
        "agents_v5_shell_policy",
        "ALTER TABLE agents ADD COLUMN shell_policy TEXT"
    ),
    // Add allowed_paths JSON column to agents
    (
        "agents_v5_allowed_paths",
        "ALTER TABLE agents ADD COLUMN allowed_paths TEXT"
    ),
    // Add final_audit boolean column to agents
    (
        "agents_v5_final_audit",
        "ALTER TABLE agents ADD COLUMN final_audit BOOLEAN DEFAULT 0"
    ),
    // Add approval_level column to agents
    (
        "agents_v5_approval_level",
        "ALTER TABLE agents ADD COLUMN approval_level TEXT DEFAULT 'default'"
    ),
];
