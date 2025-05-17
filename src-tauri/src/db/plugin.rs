//! Plugin database management module.
//!
//! This module provides functionality for managing plugins in the database, including:
//! - Plugin CRUD operations
//! - Plugin file management
//! - Runtime type conversions
//!
//! The module uses SQLite for data persistence and implements proper error handling
//! for all database operations.

use super::types::{Plugin, PluginFile, PluginListItem, RuntimeType};
use crate::db::error::StoreError;
use crate::db::main_store::MainStore;
use rusqlite::{params, OptionalExtension};
use rust_i18n::t;

impl ToString for RuntimeType {
    fn to_string(&self) -> String {
        match self {
            RuntimeType::Python => "python".to_string(),
            RuntimeType::JavaScript => "javascript".to_string(),
            RuntimeType::TypeScript => "typescript".to_string(),
        }
    }
}

impl TryFrom<String> for RuntimeType {
    type Error = StoreError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.to_lowercase().as_str() {
            "python" => Ok(RuntimeType::Python),
            "javascript" => Ok(RuntimeType::JavaScript),
            "typescript" => Ok(RuntimeType::TypeScript),
            _ => Err(StoreError::InvalidData(
                t!("db.plugin_invalid_runtime_type", runtime_type = s).to_string(),
            )),
        }
    }
}

impl MainStore {
    /// Creates a new plugin in the database.
    ///
    /// # Arguments
    ///
    /// * `plugin` - The plugin to create
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the plugin was created successfully.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if:
    /// - Failed to serialize input/output schema
    /// - Database operation failed
    pub fn create_plugin(&self, plugin: &Plugin) -> Result<(), StoreError> {
        let input_schema = plugin
            .input_schema
            .as_ref()
            .map(|v| serde_json::to_string(v))
            .transpose()
            .map_err(|e| {
                StoreError::JsonError(
                    t!(
                        "db.plugin_json_serialize_failed_input_schema",
                        error = e.to_string()
                    )
                    .to_string(),
                )
            })?;

        let output_schema = plugin
            .output_schema
            .as_ref()
            .map(|v| serde_json::to_string(v))
            .transpose()
            .map_err(|e| {
                StoreError::JsonError(
                    t!(
                        "db.plugin_json_serialize_failed_output_schema",
                        error = e.to_string()
                    )
                    .to_string(),
                )
            })?;

        self.conn.execute(
            r#"
            INSERT INTO plugins (
                uuid, name, description, author, version, runtime_type,
                input_schema, output_schema, icon, readme, checksum
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11
            )
            "#,
            params![
                plugin.uuid,
                plugin.name,
                plugin.description,
                plugin.author,
                plugin.version,
                plugin.runtime_type.to_string(),
                input_schema,
                output_schema,
                plugin.icon,
                plugin.readme,
                plugin.checksum,
            ],
        )?;

        Ok(())
    }

    /// Retrieves a plugin by its UUID.
    ///
    /// # Arguments
    ///
    /// * `uuid` - The UUID of the plugin to retrieve
    ///
    /// # Returns
    ///
    /// Returns `Ok(Some(Plugin))` if the plugin was found, `Ok(None)` if not found.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if:
    /// - Failed to deserialize input/output schema
    /// - Database operation failed
    pub fn get_plugin_by_uuid(&self, uuid: &str) -> Result<Option<Plugin>, StoreError> {
        self.conn
            .query_row(
                r#"
                SELECT * FROM plugins WHERE uuid = ?
                "#,
                [uuid],
                |row| {
                    Ok(Plugin {
                        uuid: row.get("uuid")?,
                        name: row.get("name")?,
                        description: row.get("description")?,
                        author: row.get("author")?,
                        version: row.get("version")?,
                        runtime_type: RuntimeType::try_from(row.get::<_, String>("runtime_type")?)?,
                        input_schema: row
                            .get::<_, Option<String>>("input_schema")?
                            .map(|s| serde_json::from_str(&s))
                            .transpose()
                            .map_err(|e| {
                                StoreError::JsonError(
                                    t!(
                                        "db.plugin_json_deserialize_failed_input_schema",
                                        error = e.to_string()
                                    )
                                    .to_string(),
                                )
                            })?,
                        output_schema: row
                            .get::<_, Option<String>>("output_schema")?
                            .map(|s| serde_json::from_str(&s))
                            .transpose()
                            .map_err(|e| {
                                StoreError::JsonError(
                                    t!(
                                        "db.plugin_json_deserialize_failed_output_schema",
                                        error = e.to_string()
                                    )
                                    .to_string(),
                                )
                            })?,
                        icon: row.get("icon")?,
                        readme: row.get("readme")?,
                        checksum: row.get("checksum")?,
                        created_at: row.get("created_at")?,
                        updated_at: row.get("updated_at")?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
    }

    /// Retrieves a list of all plugins.
    ///
    /// # Returns
    ///
    /// Returns `Ok(Vec<PluginListItem>)` containing all plugins in the database.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if:
    /// - Failed to prepare or execute the query
    /// - Failed to convert database rows to plugin list items
    pub fn get_plugin_list(&self) -> Result<Vec<PluginListItem>, StoreError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT uuid, name, description, author, version, runtime_type, icon, created_at, updated_at
            FROM plugins
            ORDER BY created_at DESC
            "#,
        )?;

        let plugins = stmt
            .query_map([], |row| {
                Ok(PluginListItem {
                    uuid: row.get("uuid")?,
                    name: row.get("name")?,
                    description: row.get("description")?,
                    author: row.get("author")?,
                    version: row.get("version")?,
                    runtime_type: RuntimeType::try_from(row.get::<_, String>("runtime_type")?)?,
                    icon: row.get("icon")?,
                    created_at: row.get("created_at")?,
                    updated_at: row.get("updated_at")?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)?;

        Ok(plugins)
    }

    /// Updates an existing plugin.
    ///
    /// # Arguments
    ///
    /// * `plugin` - The plugin with updated information
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the plugin was updated successfully.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if:
    /// - Plugin not found
    /// - Failed to serialize input/output schema
    /// - Database operation failed
    pub fn update_plugin(&self, plugin: &Plugin) -> Result<(), StoreError> {
        let input_schema = plugin
            .input_schema
            .as_ref()
            .map(|v| serde_json::to_string(v))
            .transpose()
            .map_err(|e| {
                StoreError::JsonError(
                    t!(
                        "db.plugin_json_serialize_failed_input_schema",
                        error = e.to_string()
                    )
                    .to_string(),
                )
            })?;

        let output_schema = plugin
            .output_schema
            .as_ref()
            .map(|v| serde_json::to_string(v))
            .transpose()
            .map_err(|e| {
                StoreError::JsonError(
                    t!(
                        "db.plugin_json_serialize_failed_output_schema",
                        error = e.to_string()
                    )
                    .to_string(),
                )
            })?;

        let affected = self.conn.execute(
            r#"
            UPDATE plugins SET
                name = ?2,
                description = ?3,
                author = ?4,
                version = ?5,
                runtime_type = ?6,
                input_schema = ?7,
                output_schema = ?8,
                icon = ?9,
                readme = ?10,
                checksum = ?11,
                updated_at = CURRENT_TIMESTAMP
            WHERE uuid = ?1
            "#,
            params![
                plugin.uuid,
                plugin.name,
                plugin.description,
                plugin.author,
                plugin.version,
                plugin.runtime_type.to_string(),
                input_schema,
                output_schema,
                plugin.icon,
                plugin.readme,
                plugin.checksum,
            ],
        )?;

        if affected == 0 {
            return Err(StoreError::NotFound(
                t!("db.plugin_not_found_by_uuid", uuid = plugin.uuid).to_string(),
            ));
        }

        Ok(())
    }

    /// Deletes a plugin and its associated files.
    ///
    /// # Arguments
    ///
    /// * `uuid` - The UUID of the plugin to delete
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the plugin was deleted successfully.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if:
    /// - Plugin not found
    /// - Database operation failed
    pub fn delete_plugin(&self, uuid: &str) -> Result<(), StoreError> {
        let affected = self
            .conn
            .execute("DELETE FROM plugins WHERE uuid = ?", [uuid])?;

        if affected == 0 {
            return Err(StoreError::NotFound(
                t!("db.plugin_not_found_by_uuid", uuid = uuid).to_string(),
            ));
        }

        Ok(())
    }

    /// Adds a new plugin file.
    ///
    /// # Arguments
    ///
    /// * `file` - The plugin file to add
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the file was added successfully.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if:
    /// - Database operation failed
    /// - Constraint violation (e.g., duplicate file for the same plugin)
    pub fn add_plugin_file(&self, file: &PluginFile) -> Result<(), StoreError> {
        self.conn.execute(
            r#"
            INSERT INTO plugin_files (
                uuid, plugin_id, filename, content, is_entry
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5
            )
            "#,
            params![
                file.uuid,
                file.plugin_id,
                file.filename,
                file.content,
                file.is_entry,
            ],
        )?;

        Ok(())
    }

    /// Updates an existing plugin file.
    ///
    /// # Arguments
    ///
    /// * `file` - The plugin file with updated information
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the file was updated successfully.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if:
    /// - File not found
    /// - Database operation failed
    pub fn update_plugin_file(&self, file: &PluginFile) -> Result<(), StoreError> {
        let affected = self.conn.execute(
            r#"
            UPDATE plugin_files SET
                filename = ?2,
                content = ?3,
                is_entry = ?4,
                updated_at = CURRENT_TIMESTAMP
            WHERE uuid = ?1
            "#,
            params![file.uuid, file.filename, file.content, file.is_entry],
        )?;

        if affected == 0 {
            return Err(StoreError::NotFound(
                t!("db.plugin_file_not_found_by_uuid", uuid = file.uuid).to_string(),
            ));
        }

        Ok(())
    }

    /// Deletes a plugin file.
    ///
    /// # Arguments
    ///
    /// * `uuid` - The UUID of the file to delete
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the file was deleted successfully.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if:
    /// - File not found
    /// - Database operation failed
    pub fn delete_plugin_file(&self, uuid: &str) -> Result<(), StoreError> {
        let affected = self
            .conn
            .execute("DELETE FROM plugin_files WHERE uuid = ?", [uuid])?;

        if affected == 0 {
            return Err(StoreError::NotFound(
                t!("db.plugin_file_not_found_by_uuid", uuid = uuid).to_string(),
            ));
        }

        Ok(())
    }

    /// Retrieves all files associated with a plugin.
    ///
    /// # Arguments
    ///
    /// * `plugin_id` - The UUID of the plugin whose files to retrieve
    ///
    /// # Returns
    ///
    /// Returns `Ok(Vec<PluginFile>)` containing all files associated with the plugin.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if:
    /// - Failed to prepare or execute the query
    /// - Failed to convert database rows to plugin files
    pub fn get_plugin_files(&self, plugin_id: &str) -> Result<Vec<PluginFile>, StoreError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT * FROM plugin_files WHERE plugin_id = ?
            "#,
        )?;

        let files = stmt
            .query_map([plugin_id], |row| {
                Ok(PluginFile {
                    uuid: row.get("uuid")?,
                    plugin_id: row.get("plugin_id")?,
                    filename: row.get("filename")?,
                    content: row.get("content")?,
                    is_entry: row.get("is_entry")?,
                    created_at: row.get("created_at")?,
                    updated_at: row.get("updated_at")?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)?;

        Ok(files)
    }
}
