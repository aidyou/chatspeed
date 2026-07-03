use super::{column_exists, MigrationDefinition};
use crate::db::StoreError;
use rusqlite::Connection;

pub const MIGRATION_SQL: &[(&str, &str)] = &[];

fn ensure_ccproxy_provider_id(conn: &Connection) -> Result<(), StoreError> {
    if !column_exists(conn, "ccproxy_stats", "provider_id")? {
        conn.execute(
            "ALTER TABLE ccproxy_stats ADD COLUMN provider_id INTEGER",
            [],
        )?;
    }

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_ccproxy_stats_provider_id ON ccproxy_stats(provider_id)",
        [],
    )?;

    conn.execute(
        "UPDATE ccproxy_stats
         SET provider_id = (
             SELECT ai_model.id
             FROM ai_model
             WHERE ai_model.name = ccproxy_stats.provider
             LIMIT 1
         )
         WHERE provider_id IS NULL",
        [],
    )?;

    Ok(())
}

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 7,
    description: "v7 migration: Add provider_id to ccproxy stats",
    sql: MIGRATION_SQL,
    ensure: Some(ensure_ccproxy_provider_id),
};
