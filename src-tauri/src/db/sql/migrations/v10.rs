use super::common::{column_exists, MigrationDefinition};
use crate::db::StoreError;
use rusqlite::Connection;

pub const MIGRATION_SQL: &[(&str, &str)] = &[];

fn ensure_mcp_tool_exposure_column(conn: &Connection) -> Result<(), StoreError> {
    if !column_exists(conn, "agents", "mcp_tool_exposure")? {
        conn.execute("ALTER TABLE agents ADD COLUMN mcp_tool_exposure TEXT", [])?;
    }
    Ok(())
}

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 10,
    description: "v10 migration: Persist per-agent direct MCP tool exposure",
    sql: MIGRATION_SQL,
    ensure: Some(ensure_mcp_tool_exposure_column),
};
