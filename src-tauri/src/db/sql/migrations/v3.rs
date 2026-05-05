use super::MigrationDefinition;

/// Version 3 migration SQL statements (Placeholder for compatibility)
pub const MIGRATION_SQL: &[(&str, &str)] = &[];

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 3,
    description: "v3 migration: Placeholder for compatibility",
    sql: MIGRATION_SQL,
    ensure: None,
};
