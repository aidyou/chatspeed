use super::common::MigrationDefinition;
use crate::db::StoreError;
use rusqlite::Connection;

/// Version 4 migration SQL statements
/// Adds Proxy Stats table for tracking proxy requests and token usage.
pub const MIGRATION_SQL: &[(&str, &str)] = &[
    // Proxy stats table
    (
        "ccproxy_stats",
        "CREATE TABLE IF NOT EXISTS ccproxy_stats (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            client_model TEXT NOT NULL,
            backend_model TEXT NOT NULL,
            provider TEXT NOT NULL,
            protocol TEXT NOT NULL,
            tool_compat_mode INTEGER DEFAULT 0,
            status_code INTEGER NOT NULL,
            error_message TEXT,
            input_tokens INTEGER DEFAULT 0,
            output_tokens INTEGER DEFAULT 0,
            cache_tokens INTEGER DEFAULT 0,
            request_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
    ),
    (
        "idx_ccproxy_stats_request_at",
        "CREATE INDEX IF NOT EXISTS idx_ccproxy_stats_request_at ON ccproxy_stats(request_at DESC)",
    ),
    (
        "idx_ccproxy_stats_provider",
        "CREATE INDEX IF NOT EXISTS idx_ccproxy_stats_provider ON ccproxy_stats(provider)",
    ),
    (
        "idx_ccproxy_stats_status_code",
        "CREATE INDEX IF NOT EXISTS idx_ccproxy_stats_status_code ON ccproxy_stats(status_code)",
    ),
];

fn ensure_ccproxy_stats_indexes(conn: &Connection) -> Result<(), StoreError> {
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_ccproxy_stats_request_at
         ON ccproxy_stats(request_at DESC)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_ccproxy_stats_provider
         ON ccproxy_stats(provider)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_ccproxy_stats_status_code
         ON ccproxy_stats(status_code)",
        [],
    )?;
    Ok(())
}

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 4,
    description: "v4 migration: Add ccproxy_stats table",
    sql: MIGRATION_SQL,
    ensure: Some(ensure_ccproxy_stats_indexes),
    apply: None,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_ccproxy_stats_indexes_idempotently() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        conn.execute_batch(
            "CREATE TABLE ccproxy_stats (
                request_at DATETIME,
                provider TEXT,
                status_code INTEGER
            );",
        )
        .expect("failed to create migration fixture");

        ensure_ccproxy_stats_indexes(&conn).expect("index ensure should succeed");
        ensure_ccproxy_stats_indexes(&conn).expect("index ensure should be idempotent");

        for index_name in [
            "idx_ccproxy_stats_request_at",
            "idx_ccproxy_stats_provider",
            "idx_ccproxy_stats_status_code",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(1) FROM sqlite_master
                     WHERE type = 'index' AND name = ?1",
                    [index_name],
                    |row| row.get(0),
                )
                .expect("failed to query ensured index");
            assert_eq!(count, 1, "expected index {index_name}");
        }
    }
}
