use crate::db::StoreError;
use rusqlite::Connection;

/// Version 2 migration SQL statements
pub const MIGRATION_SQL: &[(&str, &str)] = &[
    // (
    //     PLUGINS_TABLE,
    //     "CREATE TABLE IF NOT EXISTS plugins (
    //         uuid TEXT PRIMARY KEY,
    //         name TEXT NOT NULL,
    //         description TEXT,
    //         author TEXT NOT NULL,
    //         version TEXT NOT NULL,
    //         runtime_type TEXT NOT NULL,
    //         input_schema TEXT,
    //         output_schema TEXT,
    //         icon TEXT,
    //         readme TEXT,
    //         checksum TEXT NOT NULL,
    //         created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    //         updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
    //     )",
    // ),
    // (
    //     PLUGIN_FILES_TABLE,
    //     "CREATE TABLE IF NOT EXISTS plugin_files (
    //         uuid TEXT PRIMARY KEY,
    //         plugin_id TEXT NOT NULL REFERENCES plugins(uuid) ON DELETE CASCADE,
    //         filename TEXT NOT NULL,
    //         content TEXT NOT NULL,
    //         is_entry BOOLEAN NOT NULL DEFAULT 0,
    //         created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    //         updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    //         UNIQUE(plugin_id, filename)
    //     )",
    // ),
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
