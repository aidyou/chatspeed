use crate::db::StoreError;
use rusqlite::Connection;

pub type MigrationEnsureFn = fn(&Connection) -> Result<(), StoreError>;

pub struct MigrationDefinition {
    pub version: i32,
    pub description: &'static str,
    pub sql: &'static [(&'static str, &'static str)],
    pub ensure: Option<MigrationEnsureFn>,
}

pub(crate) fn column_exists(
    conn: &Connection,
    table: &str,
    column: &str,
) -> Result<bool, StoreError> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;

    for column_name in columns {
        if column_name? == column {
            return Ok(true);
        }
    }

    Ok(false)
}

pub mod manager;
pub mod v1;
pub mod v2;
pub mod v3;
pub mod v4;
pub mod v5;
