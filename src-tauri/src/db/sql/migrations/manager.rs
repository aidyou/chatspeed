use crate::db::sql::migrations::{v1, v2};
use crate::db::StoreError;
use rusqlite::Connection;

// Define the migration structure to hold SQL statements directly.
struct Migration {
    version: i32,
    description: &'static str,
    sql: &'static [(&'static str, &'static str)],
}

// Register all migrations with their corresponding SQL.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 2,
        description: "v2 migration: Add agents and workflows tables",
        sql: v2::MIGRATION_SQL,
    },
    // Migration {
    //     version: 3,
    //     description: "v3 migration: Add workflows table",
    //     sql: v3::MIGRATION_SQL,
    // },
];

/// Executes a given set of SQL statements within a transaction and updates the db version.
fn execute_migration_sql(
    conn: &mut Connection,
    sql_statements: &[(&str, &str)],
    version: i32,
) -> Result<(), StoreError> {
    if sql_statements.is_empty() {
        return Ok(());
    }

    let tx = conn.transaction()?;
    for (_name, sql) in sql_statements {
        tx.execute(sql, [])?;
    }

    // Insert or replace the database version.
    tx.execute(
        "INSERT OR REPLACE INTO db_version (version) VALUES (?1)",
        [version],
    )?;

    tx.commit()?;
    Ok(())
}

/// Gets the current database version
pub fn get_db_version(conn: &Connection) -> Result<i32, StoreError> {
    let version: i32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM db_version",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    Ok(version)
}

/// Runs all necessary migrations to update the database to the latest version
pub fn run_migrations(conn: &mut Connection) -> Result<(), StoreError> {
    let mut current_version = get_db_version(conn)?;

    // If it's a fresh install (version 0), run the v1 initialization.
    if current_version < 1 {
        log::info!("Database not initialized. Running initial migration (v1)...");
        execute_migration_sql(conn, v1::INIT_SQL, 1)?;
        current_version = get_db_version(conn)?;
        log::info!("Initial migration to v{} complete.", current_version);
    }

    // Determine the latest version available from our registered migrations.
    let latest_migration_version = MIGRATIONS.last().map_or(1, |m| m.version);

    if current_version >= latest_migration_version {
        log::info!(
            "Database is already up to date at version {}.",
            current_version
        );
        return Ok(());
    }

    log::info!(
        "Current DB version: {}. Checking for pending migrations...",
        current_version
    );

    // Filter out all migrations that need to be executed and sort them by version.
    let mut pending_migrations: Vec<&Migration> = MIGRATIONS
        .iter()
        .filter(|m| m.version > current_version)
        .collect();
    pending_migrations.sort_by_key(|m| m.version);

    // Execute all pending migrations in order.
    for migration in pending_migrations {
        log::info!(
            "Applying migration version {}... ({})",
            migration.version,
            migration.description
        );
        execute_migration_sql(conn, migration.sql, migration.version)?;
        log::info!(
            "Successfully applied migration version {}.",
            migration.version
        );
    }

    let final_version = get_db_version(conn)?;
    log::info!(
        "All migrations applied. Database is now at version {}.",
        final_version
    );

    Ok(())
}
