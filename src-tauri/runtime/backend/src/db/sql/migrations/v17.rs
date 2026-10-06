use super::common::MigrationDefinition;
use crate::db::StoreError;
use rusqlite::Connection;

pub const MIGRATION_SQL: &[(&str, &str)] = &[
    (
        "cache_write_tokens",
        "ALTER TABLE ccproxy_stats ADD COLUMN cache_write_tokens INTEGER NOT NULL DEFAULT 0",
    ),
    (
        "reasoning_tokens",
        "ALTER TABLE ccproxy_stats ADD COLUMN reasoning_tokens INTEGER NOT NULL DEFAULT 0",
    ),
    (
        "audio_input_tokens",
        "ALTER TABLE ccproxy_stats ADD COLUMN audio_input_tokens INTEGER NOT NULL DEFAULT 0",
    ),
    (
        "audio_output_tokens",
        "ALTER TABLE ccproxy_stats ADD COLUMN audio_output_tokens INTEGER NOT NULL DEFAULT 0",
    ),
    (
        "estimated_cost",
        "ALTER TABLE ccproxy_stats ADD COLUMN estimated_cost REAL",
    ),
    (
        "pricing_status",
        "ALTER TABLE ccproxy_stats ADD COLUMN pricing_status TEXT",
    ),
    (
        "pricing_snapshot",
        "ALTER TABLE ccproxy_stats ADD COLUMN pricing_snapshot TEXT",
    ),
];

fn ensure_columns(conn: &Connection) -> Result<(), StoreError> {
    for (name, sql) in MIGRATION_SQL {
        let exists = conn
            .prepare("SELECT 1 FROM pragma_table_info('ccproxy_stats') WHERE name = ?1")?
            .exists([*name])?;
        if !exists {
            conn.execute(sql, [])?;
        }
    }
    Ok(())
}

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 17,
    description: "v17 migration: Add canonical usage dimensions and pricing audit fields",
    sql: &[],
    ensure: Some(ensure_columns),
    apply: None,
};
