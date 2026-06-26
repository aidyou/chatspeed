use super::MigrationDefinition;

/// Version 6 migration SQL statements.
/// Adds workflow automation definitions and run history.
pub const MIGRATION_SQL: &[(&str, &str)] = &[
    (
        "workflow_automations",
        "CREATE TABLE IF NOT EXISTS workflow_automations (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            prompt TEXT,
            prompt_file_path TEXT,
            agent_id TEXT NOT NULL REFERENCES agents(id),
            agent_config TEXT,
            allowed_paths TEXT NOT NULL DEFAULT '[]',
            schedule_kind TEXT NOT NULL,
            schedule_config TEXT NOT NULL,
            self_review INTEGER NOT NULL DEFAULT 0,
            enabled INTEGER NOT NULL DEFAULT 1,
            next_run_at DATETIME,
            last_run_at DATETIME,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
    ),
    (
        "idx_workflow_automations_enabled_next_run_at",
        "CREATE INDEX IF NOT EXISTS idx_workflow_automations_enabled_next_run_at
         ON workflow_automations(enabled, next_run_at)",
    ),
    (
        "idx_workflow_automations_updated_at",
        "CREATE INDEX IF NOT EXISTS idx_workflow_automations_updated_at
         ON workflow_automations(updated_at DESC)",
    ),
    (
        "workflow_automation_runs",
        "CREATE TABLE IF NOT EXISTS workflow_automation_runs (
            id TEXT PRIMARY KEY,
            automation_id TEXT NOT NULL REFERENCES workflow_automations(id),
            workflow_session_id TEXT REFERENCES workflows(id),
            status TEXT NOT NULL,
            scheduled_for DATETIME NOT NULL,
            started_at DATETIME,
            finished_at DATETIME,
            error TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
    ),
    (
        "idx_workflow_automation_runs_automation_created_at",
        "CREATE INDEX IF NOT EXISTS idx_workflow_automation_runs_automation_created_at
         ON workflow_automation_runs(automation_id, created_at DESC)",
    ),
    (
        "idx_workflow_automation_runs_workflow_session_id",
        "CREATE INDEX IF NOT EXISTS idx_workflow_automation_runs_workflow_session_id
         ON workflow_automation_runs(workflow_session_id)",
    ),
];

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 6,
    description: "v6 migration: Add workflow automation tables",
    sql: MIGRATION_SQL,
    ensure: None,
};
