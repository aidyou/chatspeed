use super::common::MigrationDefinition;

pub const MIGRATION_SQL: &[(&str, &str)] = &[(
    "idx_workflow_context_messages_source_message_id",
    "CREATE INDEX IF NOT EXISTS idx_workflow_context_messages_source_message_id
     ON workflow_context_messages(source_message_id)",
)];

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 9,
    description: "v9 migration: Index workflow context source messages",
    sql: MIGRATION_SQL,
    ensure: None,
};
