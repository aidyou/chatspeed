use crate::db::StoreError;
use rusqlite::Connection;
use rust_i18n::t;

/// Gets the current database version
pub fn get_db_version(conn: &Connection) -> Result<i32, StoreError> {
    // Get current database version.
    // When a user enters for the first time, an uninstalled database could cause a panic during retrieval.
    // Therefore, we should use 0 as the default version, allowing the manager to initialize properly.
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
    let current_version = get_db_version(conn)?;

    // if current version is 0, run v1 migration
    if current_version == 0 {
        crate::db::sql::migrations::v1::run_migration(conn)?;
        return Ok(());
    }

    // run migrations by current version
    match current_version {
        1 => {
            // run v2 migration
            crate::db::sql::migrations::v2::run_migration(conn)?;
        }
        2 => {
            // current version is already latest
        }
        _ => {
            let error_message =
                t!("unknown_database_version", version = current_version).to_string();
            log::error!("{}", error_message);
            return Err(StoreError::DatabaseError(error_message));
        }
    }

    Ok(())
}
