use super::common::{column_exists, MigrationDefinition};
use crate::db::StoreError;
use rusqlite::Connection;

const MODE_COLUMN: &str = "mode";

fn ensure_mode_column(conn: &Connection) -> Result<(), StoreError> {
    if !column_exists(conn, "agents", MODE_COLUMN)? {
        conn.execute("ALTER TABLE agents ADD COLUMN mode TEXT", [])?;
    }
    Ok(())
}

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 19,
    description: "v19 migration: Add nullable agent execution mode",
    sql: &[],
    ensure: Some(ensure_mode_column),
    apply: None,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_nullable_mode_column_idempotently() {
        let conn = Connection::open_in_memory().expect("open database");
        conn.execute("CREATE TABLE agents (id TEXT PRIMARY KEY)", [])
            .expect("create agents");
        ensure_mode_column(&conn).expect("add mode");
        ensure_mode_column(&conn).expect("ensure mode");
        let (count, not_null): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), MAX(\"notnull\") FROM pragma_table_info('agents') WHERE name = 'mode'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("inspect mode");
        assert_eq!(count, 1);
        assert_eq!(not_null, 0);
    }
}
