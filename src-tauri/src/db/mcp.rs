//! Module for managing MCP (Model Context Protocol) records in SQLite database.
//!
//! Provides CRUD operations for MCP records with JSON serialization/deserialization.

use rusqlite::{params, Connection};

use super::{MainStore, StoreError};
use crate::mcp::client::{McpServerConfig, McpStatus};
use rust_i18n::t;

/// Represents a Model Context Protocol (MCP) record
///
/// # Fields
/// - `id`: Unique identifier for the MCP record
/// - `name`: Human-readable name of the MCP configuration
/// - `description`: Detailed description of the MCP configuration
/// - `config`: Server configuration in JSON format
/// - `disable`: Whether this MCP configuration is disabled
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Mcp {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub config: McpServerConfig,
    pub disabled: bool,
    pub status: Option<McpStatus>,
}

impl MainStore {
    /// Retrieves all MCP records from the database
    ///
    /// # Returns
    /// Returns `Result` with a vector of `Mcp` structs on success, or `StoreError` on failure
    ///
    /// # Errors
    /// Returns `StoreError` if:
    /// - JSON deserialization fails
    /// - SQL execution fails
    pub fn get_all_mcps(conn: &Connection) -> Result<Vec<Mcp>, StoreError> {
        let mut stmt = conn.prepare("SELECT id, name, description, config, disabled FROM mcp")?;
        let rows = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let name: String = row.get(1)?;
            let description: String = row.get(2)?;
            let config_json: String = row.get(3)?;
            let disable: bool = row.get(4)?;

            let config: McpServerConfig = serde_json::from_str(&config_json).map_err(|e| {
                log::error!(
                    "Failed to parse MCP config JSON for MCP (id: {} name: {}): {}, error: {}",
                    id,
                    name,
                    config_json,
                    e
                );
                StoreError::JsonError(
                    t!("db.json_parse_failed_mcp_config", error = e.to_string()).to_string(),
                )
            })?;

            Ok(Mcp {
                id,
                name,
                description,
                config,
                disabled: disable,
                status: Some(McpStatus::Stopped),
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| StoreError::from(e))
    }

    /// Creates a new MCP record in the database
    ///
    /// # Arguments
    /// * `name` - Name of the MCP configuration
    /// * `description` - Description of the MCP configuration
    /// * `config` - Server configuration object
    /// * `disabled` - Initial disabled state
    ///
    /// # Returns
    /// Returns `Result` with the new record ID on success, or `StoreError` on failure
    ///
    /// # Errors
    /// Returns `StoreError` if:
    /// - JSON serialization fails
    /// - SQL execution fails
    pub fn add_mcp(
        &mut self,
        name: String,
        description: String,
        config: McpServerConfig,
        disabled: bool,
    ) -> Result<Mcp, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::FailedToLockMainStore(e.to_string()))?;
        let config_json = serde_json::to_string(&config).map_err(|e| {
            StoreError::JsonError(
                t!("db.json_serialize_failed_mcp_config", error = e.to_string()).to_string(),
            )
        })?;

        conn.execute(
            "INSERT INTO mcp (name, description, config, disabled) VALUES (?1, ?2, ?3, ?4)",
            params![name, description, config_json, disabled],
        )?;

        let id = conn.last_insert_rowid();
        if let Ok(mcp) = Self::get_all_mcps(&conn) {
            self.config.set_mcps(mcp);
        }

        self.config.get_mcp_by_id(id)
    }

    /// Updates an existing MCP record
    ///
    /// # Arguments
    /// * `id` - ID of the record to update
    /// * `name` - New name for the MCP configuration
    /// * `description` - New description
    /// * `config` - Updated server configuration
    /// * `disable` - New disabled state
    ///
    /// # Returns
    /// Returns `Result` with unit type `()` on success, or `StoreError` on failure
    ///
    /// # Errors
    /// Returns `StoreError` if:
    /// - JSON serialization fails
    /// - SQL execution fails
    pub fn update_mcp(
        &mut self,
        id: i64,
        name: &str,
        description: &str,
        config: McpServerConfig,
        disable: bool,
    ) -> Result<Mcp, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::FailedToLockMainStore(e.to_string()))?;
        let config_json = serde_json::to_string(&config).map_err(|e| {
            StoreError::JsonError(
                t!("db.json_serialize_failed_mcp_config", error = e.to_string()).to_string(),
            )
        })?;

        conn.execute(
            "UPDATE mcp SET
                name = ?,
                description = ?,
                config = ?,
                disabled = ?
             WHERE id = ?",
            params![name, description, config_json, disable, id],
        )?;

        if let Ok(mcp) = Self::get_all_mcps(&conn) {
            self.config.set_mcps(mcp);
        }

        self.config.get_mcp_by_id(id)
    }

    /// Deletes an MCP record by its ID
    ///
    /// # Arguments
    /// * `id` - ID of the record to delete
    ///
    /// # Returns
    /// Returns `Result` with unit type `()` on success, or `StoreError` on failure
    ///
    /// # Errors
    /// Returns `StoreError` if:
    /// - SQL execution fails
    /// - Transaction commit fails
    pub fn delete_mcp(&mut self, id: i64) -> Result<(), StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::FailedToLockMainStore(e.to_string()))?;
        conn.execute("DELETE FROM mcp WHERE id = ?", params![id])?;

        if let Ok(mcp) = Self::get_all_mcps(&conn) {
            self.config.set_mcps(mcp);
        }
        Ok(())
    }

    /// Updates the status of an MCP record
    ///
    /// # Arguments
    /// * `id` - ID of the record to update
    /// * `disabled` - New disabled state
    ///
    /// # Returns
    /// Returns `Result` with unit type `()` on success, or `StoreError` on failure
    pub fn change_mcp_status(&mut self, id: i64, disabled: bool) -> Result<Mcp, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::FailedToLockMainStore(e.to_string()))?;
        conn.execute(
            "UPDATE mcp SET disabled =? WHERE id =?",
            params![disabled, id],
        )?;

        if let Ok(mcp) = Self::get_all_mcps(&conn) {
            self.config.set_mcps(mcp);
        }

        self.config.get_mcp_by_id(id)
    }
}
