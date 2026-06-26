use crate::db::sql::migrations::{v1, v2, v3, v4, v5, v6, MigrationDefinition};
use crate::db::StoreError;
use rusqlite::Connection;

// Register all migrations with their corresponding SQL.
const MIGRATIONS: &[MigrationDefinition] = &[
    v2::MIGRATION,
    v3::MIGRATION,
    v4::MIGRATION,
    v5::MIGRATION,
    v6::MIGRATION,
];

fn latest_migration_version() -> i32 {
    MIGRATIONS.last().map_or(1, |migration| migration.version)
}

fn latest_schema_statements() -> Vec<(&'static str, &'static str)> {
    let mut statements = Vec::new();
    statements.extend_from_slice(v1::INIT_SQL);
    for migration in MIGRATIONS {
        statements.extend_from_slice(migration.sql);
    }
    statements
}

/// Executes a given set of SQL statements within a transaction and updates the db version.
fn execute_migration_statements<I>(
    conn: &mut Connection,
    sql_statements: I,
    version: i32,
) -> Result<(), StoreError>
where
    I: IntoIterator<Item = (&'static str, &'static str)>,
{
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

fn run_post_migration_ensures(conn: &Connection, current_version: i32) -> Result<(), StoreError> {
    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version <= current_version)
    {
        if let Some(ensure) = migration.ensure {
            ensure(conn)?;
        }
    }

    Ok(())
}

/// Gets the current database version
pub fn get_db_version(conn: &Connection) -> Result<i32, StoreError> {
    let result: Result<i32, rusqlite::Error> = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM db_version",
        [],
        |row| row.get(0),
    );

    match result {
        Ok(v) => Ok(v),
        Err(e) => {
            if e.to_string().contains("no such table: db_version") {
                Ok(0)
            } else {
                Err(StoreError::from(e))
            }
        }
    }
}

/// Runs all necessary migrations to update the database to the latest version
pub fn run_migrations(conn: &mut Connection) -> Result<(), StoreError> {
    let mut current_version = get_db_version(conn)?;
    let latest_version = latest_migration_version();

    // A brand-new database should install the latest schema in one pass instead of
    // replaying every historical migration step.
    if current_version < 1 {
        log::info!(
            "Database not initialized. Installing latest schema at version {}...",
            latest_version
        );
        execute_migration_statements(conn, latest_schema_statements(), latest_version)?;
        current_version = get_db_version(conn)?;
        log::info!(
            "Fresh database installation complete at v{}.",
            current_version
        );

        run_post_migration_ensures(conn, current_version)?;
        return Ok(());
    }

    if current_version >= latest_version {
        log::info!(
            "Database is already up to date at version {}.",
            current_version
        );
        run_post_migration_ensures(conn, current_version)?;
        return Ok(());
    }

    log::info!(
        "Current DB version: {}. Checking for pending migrations...",
        current_version
    );

    // Filter out all migrations that need to be executed and sort them by version.
    let mut pending_migrations: Vec<&MigrationDefinition> = MIGRATIONS
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
        execute_migration_statements(conn, migration.sql.iter().copied(), migration.version)?;
        log::info!(
            "Successfully applied migration version {}.",
            migration.version
        );
    }

    let final_version = get_db_version(conn)?;
    run_post_migration_ensures(conn, final_version)?;
    log::info!(
        "All migrations applied. Database is now at version {}.",
        final_version
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_exists(conn: &Connection, table_name: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(1) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table_name],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count == 1)
        .expect("failed to query sqlite_master for table existence")
    }

    #[test]
    fn fresh_install_builds_latest_schema_directly() {
        let mut conn = Connection::open_in_memory().expect("failed to open sqlite connection");

        run_migrations(&mut conn).expect("fresh install migrations should succeed");

        assert_eq!(
            get_db_version(&conn).expect("db version should be readable"),
            latest_migration_version()
        );
        assert!(table_exists(&conn, "agents"));
        assert!(table_exists(&conn, "workflows"));
        assert!(table_exists(&conn, "workflow_events"));
        assert!(table_exists(&conn, "memory_candidates"));

        let recorded_versions: i64 = conn
            .query_row("SELECT COUNT(1) FROM db_version", [], |row| row.get(0))
            .expect("failed to count db_version rows");
        assert_eq!(
            recorded_versions, 1,
            "fresh installs should record only the latest schema version"
        );
    }

    #[test]
    fn existing_database_upgrades_incrementally() {
        let mut conn = Connection::open_in_memory().expect("failed to open sqlite connection");

        execute_migration_statements(&mut conn, v1::INIT_SQL.iter().copied(), 1)
            .expect("v1 bootstrap should succeed");
        assert_eq!(
            get_db_version(&conn).expect("db version should be readable"),
            1
        );

        run_migrations(&mut conn).expect("incremental migrations should succeed");

        assert_eq!(
            get_db_version(&conn).expect("db version should be readable"),
            latest_migration_version()
        );
        assert!(table_exists(&conn, "agents"));
        assert!(table_exists(&conn, "ccproxy_stats"));
        assert!(table_exists(&conn, "workflows"));
        assert!(table_exists(&conn, "workflow_context_messages"));

        let has_v3_marker: i64 = conn
            .query_row(
                "SELECT COUNT(1) FROM db_version WHERE version = 3",
                [],
                |row| row.get(0),
            )
            .expect("failed to query placeholder migration marker");
        assert_eq!(
            has_v3_marker, 1,
            "placeholder migrations should still advance db_version"
        );
    }
}
