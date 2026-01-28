/// Version 5 migration SQL statements
/// Predefined Workflow tables (Currently inactive)
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
            metadata TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (session_id) REFERENCES workflows(id)
        )"
    ),
    (
        "idx_workflow_messages_session_id",
        "CREATE INDEX IF NOT EXISTS idx_workflow_messages_session_id ON workflow_messages(session_id)"
    ),
];
