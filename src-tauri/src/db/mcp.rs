//! Module for managing MCP (Model Context Protocol) records in SQLite database.
//!
//! Provides CRUD operations for MCP records with JSON serialization/deserialization.

use rusqlite::{params, Connection};

use crate::mcp::client::McpServerConfig;

use super::{MainStore, StoreError};

/// Represents a Model Context Protocol (MCP) record
///
/// # Fields
/// - `id`: Unique identifier for the MCP record
/// - `name`: Human-readable name of the MCP configuration
/// - `description`: Detailed description of the MCP configuration
/// - `config`: Server configuration in JSON format
/// - `disable`: Whether this MCP configuration is disabled
/// - `disabled_tools`: List of disabled tools for this configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Mcp {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub config: McpServerConfig,
    pub disable: bool,
    #[serde(rename = "disabledTools")]
    pub disabled_tools: Vec<String>,
}

impl MainStore {
    /// Retrieves an MCP record by its ID
    ///
    /// # Arguments
    /// * `id` - ID of the MCP record to retrieve
    ///
    /// # Returns
    /// Returns `Result` with the `Mcp` struct on success, or `StoreError` on failure
    ///
    /// # Errors
    /// Returns `StoreError` if:
    /// - Record not found
    /// - JSON deserialization fails
    /// - SQL execution fails
    pub fn get_mcp(&self, id: i64) -> Result<Mcp, StoreError> {
        self.conn.query_row(
            "SELECT id, name, description, config, disabled, disabled_tools FROM mcp WHERE id = ?",
            [id],
            |row| {
                let config_json: String = row.get("config")?;
                let disabled_tools_json: Option<String> = row.get("disabled_tools")?;

                let config: McpServerConfig = serde_json::from_str(&config_json)
                    .map_err(|e| StoreError::from(e))?;
                let disabled_tools = disabled_tools_json
                    .map(|json| serde_json::from_str(&json))
                    .transpose()
                    .map_err(|e| StoreError::JsonError(e.to_string()))?;

                Ok(Mcp {
                    id: row.get("id")?,
                    name: row.get("name")?,
                    description: row.get("description")?,
                    config,
                    disable: row.get("disabled")?,
                    disabled_tools: disabled_tools.unwrap_or_default(),
                })
            },
        )
        .map_err(|e| {
            if e == rusqlite::Error::QueryReturnedNoRows {
                StoreError::NotFound("MCP record not found".to_string())
            } else {
                StoreError::from(e)
            }
        })
    }

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
        let mut stmt = conn
            .prepare("SELECT id, name, description, config, disabled, disabled_tools FROM mcp")?;
        let rows = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let name: String = row.get(1)?;
            let description: String = row.get(2)?;
            let config_json: String = row.get(3)?;
            let disable: bool = row.get(4)?;
            let disabled_tools_json: Option<String> = row.get(5)?;

            let config: McpServerConfig =
                serde_json::from_str(&config_json).map_err(|e| StoreError::from(e))?;
            let disabled_tools = disabled_tools_json
                .map(|json| serde_json::from_str(&json))
                .transpose()
                .map_err(|e| StoreError::JsonError(e.to_string()))?;

            Ok(Mcp {
                id,
                name,
                description,
                config,
                disable,
                disabled_tools: disabled_tools.unwrap_or_default(),
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
    /// * `disabled_tools` - Optional list of disabled tools
    ///
    /// # Returns
    /// Returns `Result` with the new record ID on success, or `StoreError` on failure
    ///
    /// # Errors
    /// Returns `StoreError` if:
    /// - JSON serialization fails
    /// - SQL execution fails
    pub fn add_mcp(
        &self,
        name: String,
        description: String,
        config: McpServerConfig,
        disabled: bool,
        disabled_tools: Option<Vec<String>>,
    ) -> Result<i64, StoreError> {
        let config_json =
            serde_json::to_string(&config).map_err(|e| StoreError::JsonError(e.to_string()))?;
        let disabled_tools_json = disabled_tools
            .as_ref()
            .map(|tools| serde_json::to_string(tools))
            .transpose()
            .map_err(|e| StoreError::JsonError(e.to_string()))?;

        self.conn.execute(
            "INSERT INTO mcp (name, description, config, disabled, disabled_tools) VALUES (?1, ?2, ?3, ?4,?5)",
            params![name, description, config_json,disabled, disabled_tools_json],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    /// Updates an existing MCP record
    ///
    /// # Arguments
    /// * `id` - ID of the record to update
    /// * `name` - New name for the MCP configuration
    /// * `description` - New description
    /// * `config` - Updated server configuration
    /// * `disable` - New disabled state
    /// * `disabled_tools` - Updated list of disabled tools
    ///
    /// # Returns
    /// Returns `Result` with unit type `()` on success, or `StoreError` on failure
    ///
    /// # Errors
    /// Returns `StoreError` if:
    /// - JSON serialization fails
    /// - SQL execution fails
    pub fn update_mcp(
        &self,
        id: i64,
        name: String,
        description: String,
        config: McpServerConfig,
        disable: bool,
        disabled_tools: Option<Vec<String>>,
    ) -> Result<(), StoreError> {
        let config_json =
            serde_json::to_string(&config).map_err(|e| StoreError::JsonError(e.to_string()))?;

        let disabled_tools_json = disabled_tools
            .map(|tools| serde_json::to_string(&tools))
            .transpose()
            .map_err(|e| StoreError::JsonError(e.to_string()))?;

        self.conn.execute(
            "UPDATE mcp SET
                name = ?,
                description = ?,
                config = ?,
                disabled = ?,
                disabled_tools = ?
             WHERE id = ?",
            params![
                name,
                description,
                config_json,
                disable,
                disabled_tools_json,
                id
            ],
        )?;

        Ok(())
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
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM mcp WHERE id = ?", params![id])?;
        tx.commit()?;
        Ok(())
    }
}
