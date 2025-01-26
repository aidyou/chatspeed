//! SQLite Store Plugin Module
//!
//! This module implements a plugin that provides SQLite database operations through the Plugin trait.
//! It serves as a bridge between the core database operations and the plugin system.
//!
//! The plugin handles the serialization and deserialization of database operations,
//! converting between JSON values and strongly-typed operations.
//!
//! # Examples
//!
//! ```rust
//! use crate::plugins::core::store::{StorePlugin, types::DbOperation};
//!
//! // Create a new plugin instance
//! let plugin = StorePlugin::new("test.db")?;
//!
//! // Create a table
//! let create_operation = DbOperation::Query {
//!     sql: "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)".to_string(),
//!     params: None,
//! };
//! plugin.execute(Some(serde_json::to_value(create_operation)?),None)?;
//!
//! // Insert data
//! let mut data = serde_json::Map::new();
//! data.insert("name".to_string(), serde_json::Value::String("John".to_string()));
//!
//! let insert_operation = DbOperation::Insert {
//!     table: "users".to_string(),
//!     data,
//! };
//! let result = plugin.execute(Some(serde_json::to_value(insert_operation)?),None)?;
//! let result: types::DbResult = serde_json::from_value(result)?;
//! assert!(result.last_insert_id.is_some());
//! ```

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{core::Store, types::DbOperation};
use crate::plugins::{
    traits::{PluginFactory, PluginInfo, PluginType},
    Plugin, PluginError,
};
use async_trait::async_trait;
use rust_i18n::t;
use serde::Deserialize;
use serde_json::Value;

/// SQLite store plugin
pub struct StorePlugin {
    plugin_info: crate::plugins::traits::PluginInfo,
    store: Store,
}

impl StorePlugin {
    /// Creates a new store plugin instance
    pub fn new(db_path: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Self {
            plugin_info: PluginInfo {
                id: "store".to_string(),
                name: "store".to_string(),
                version: "1.0.0".to_string(),
            },
            store: Store::new(db_path)?,
        })
    }
}

#[async_trait]
impl Plugin for StorePlugin {
    async fn init(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn destroy(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    fn plugin_info(&self) -> &PluginInfo {
        &self.plugin_info
    }

    fn plugin_type(&self) -> &PluginType {
        &PluginType::Native
    }

    fn input_schema(&self) -> Value {
        Value::Null
    }

    fn output_schema(&self) -> Value {
        Value::Null
    }

    /// Executes a database operation
    ///
    /// # Arguments
    /// * `input` - The input value for the operation
    /// * `info` - The plugin information, which is not used in this Native plugin
    ///
    /// # Returns
    /// * `Result<Value, Box<dyn std::error::Error + Send + Sync>>` - The result of the operation
    ///
    /// # Errors
    /// * `Box<dyn std::error::Error + Send + Sync>` - If the operation fails
    async fn execute(
        &mut self,
        input: Option<Value>,
        _info: Option<PluginInfo>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let operation: DbOperation = serde_json::from_value(input.ok_or_else(|| {
            Box::new(PluginError::InvalidInput(
                t!("store.missing_operation").to_string(),
            )) as Box<dyn std::error::Error + Send + Sync>
        })?)?;

        let result = self.store.handle_operation(operation)?;
        serde_json::to_value(result).map_err(|e| {
            Box::new(PluginError::RuntimeError(
                t!("store.serialize_failed", error = e.to_string()).to_string(),
            )) as Box<dyn std::error::Error + Send + Sync>
        })
    }
}

#[derive(Deserialize)]
struct StoreOptions {
    plugin_id: String,
    db_filename: String,
}

/// Sanitize the database filename
///
/// # Arguments
/// * `filename` - The filename to sanitize
///
/// # Returns
/// * `String` - The sanitized filename
fn sanitize_filename(filename: &str) -> String {
    Path::new(filename)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|s| {
            if s.ends_with(".db") {
                s.to_string()
            } else {
                format!("{}.db", s)
            }
        })
        .unwrap_or("plugin_store.db".to_string())
}

/// Factory for creating Store plugin instances
pub struct StorePluginFactory;

impl StorePluginFactory {
    /// Creates a new store plugin factory
    pub fn new() -> Self {
        Self {}
    }
}

impl PluginFactory for StorePluginFactory {
    /// Creates a new plugin instance
    ///
    /// # Arguments
    /// * `init_options` - The initialization options for the plugin
    ///     * `plugin_id` - The ID of the plugin
    ///     * `db_filename` - The filename of the database
    ///
    /// # Returns
    /// * `Result<Box<dyn Plugin>, Box<dyn std::error::Error + Send + Sync>>` - The result of the operation
    ///
    fn create_instance(
        &self,
        init_options: Option<&Value>,
    ) -> Result<Box<dyn Plugin>, Box<dyn std::error::Error + Send + Sync>> {
        let options = if let Some(opts) = init_options {
            serde_json::from_value::<StoreOptions>(opts.clone()).map_err(|_| {
                PluginError::InvalidInput(t!("store.missing_init_options").to_string())
            })?
        } else {
            return Err(Box::new(PluginError::InvalidInput(
                t!("store.missing_init_options").to_string(),
            )));
        };
        if options.plugin_id.is_empty() {
            return Err(Box::new(PluginError::InvalidInput(
                t!("store.missing_plugin_id").to_string(),
            )));
        }
        if options.db_filename.is_empty() {
            return Err(Box::new(PluginError::InvalidInput(
                t!("store.missing_db_filename").to_string(),
            )));
        }

        let plugin_dir = crate::PLUGINS_DIR.read().clone();
        let store_dir = &mut PathBuf::from(&plugin_dir).join(options.plugin_id);
        fs::create_dir_all(&store_dir).map_err(|e| {
            PluginError::IoError(
                t!(
                    "store.failed_to_create_directory",
                    path = store_dir.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;
        let db_name = store_dir.join(options.db_filename).display().to_string();
        Ok(Box::new(StorePlugin::new(&db_name)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        constants::HTTP_SERVER_TMP_DIR,
        plugins::core::store::types::{ColumnDef, DbResult},
    };
    use std::path::Path;

    rust_i18n::i18n!("../../../../i18n");

    fn create_test_db() -> StorePlugin {
        let tmp_dir = HTTP_SERVER_TMP_DIR.read().clone();
        let db_path = Path::new(&tmp_dir).join("test.db");
        StorePlugin::new(db_path.to_str().unwrap()).unwrap()
    }

    #[tokio::test]
    async fn test_create_and_drop_table() {
        let mut plugin = create_test_db();

        // Create table
        let create_operation = DbOperation::CreateTable {
            table: "users".to_string(),
            columns: vec![
                ColumnDef {
                    name: "id".to_string(),
                    type_name: "INTEGER".to_string(),
                    constraints: vec!["PRIMARY KEY".to_string()],
                },
                ColumnDef {
                    name: "name".to_string(),
                    type_name: "TEXT".to_string(),
                    constraints: vec!["NOT NULL".to_string()],
                },
            ],
        };

        let result = plugin
            .execute(Some(serde_json::to_value(create_operation).unwrap()), None)
            .await
            .unwrap();
        let result: DbResult = serde_json::from_value(result).unwrap();
        assert_eq!(result.affected_rows, Some(0));

        // Drop table
        let drop_operation = DbOperation::DropTable {
            table: "users".to_string(),
        };

        let result = plugin
            .execute(Some(serde_json::to_value(drop_operation).unwrap()), None)
            .await
            .unwrap();
        let result: DbResult = serde_json::from_value(result).unwrap();
        assert_eq!(result.affected_rows, Some(0));
    }

    #[tokio::test]
    async fn test_crud_operations() {
        let mut plugin = create_test_db();

        // Create table
        let create_operation = DbOperation::CreateTable {
            table: "users".to_string(),
            columns: vec![
                ColumnDef {
                    name: "id".to_string(),
                    type_name: "INTEGER".to_string(),
                    constraints: vec!["PRIMARY KEY".to_string()],
                },
                ColumnDef {
                    name: "name".to_string(),
                    type_name: "TEXT".to_string(),
                    constraints: vec!["NOT NULL".to_string()],
                },
            ],
        };
        plugin
            .execute(Some(serde_json::to_value(create_operation).unwrap()), None)
            .await
            .unwrap();

        // Insert
        let mut data = serde_json::Map::new();
        data.insert("name".to_string(), Value::String("John".to_string()));

        let insert_operation = DbOperation::Insert {
            table: "users".to_string(),
            data,
        };

        let result = plugin
            .execute(Some(serde_json::to_value(insert_operation).unwrap()), None)
            .await
            .unwrap();
        let result: DbResult = serde_json::from_value(result).unwrap();
        assert_eq!(result.last_insert_id, Some(1));

        // Select
        let select_operation = DbOperation::Select {
            table: "users".to_string(),
            columns: vec!["id".to_string(), "name".to_string()],
            where_clause: Some("id = ?".to_string()),
            params: Some(vec![Value::Number(1.into())]),
        };

        let result = plugin
            .execute(Some(serde_json::to_value(select_operation).unwrap()), None)
            .await
            .unwrap();
        let result: DbResult = serde_json::from_value(result).unwrap();
        let rows = result.rows.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["name"], Value::String("John".to_string()));

        // Update
        let mut data = serde_json::Map::new();
        data.insert("name".to_string(), Value::String("Jane".to_string()));

        let update_operation = DbOperation::Update {
            table: "users".to_string(),
            data,
            where_clause: "id = ?".to_string(),
            params: Some(vec![Value::Number(1.into())]),
        };

        let result = plugin
            .execute(Some(serde_json::to_value(update_operation).unwrap()), None)
            .await
            .unwrap();
        let result: DbResult = serde_json::from_value(result).unwrap();
        assert_eq!(result.affected_rows, Some(1));

        // Delete
        let delete_operation = DbOperation::Delete {
            table: "users".to_string(),
            where_clause: "id = ?".to_string(),
            params: Some(vec![Value::Number(1.into())]),
        };

        let result = plugin
            .execute(Some(serde_json::to_value(delete_operation).unwrap()), None)
            .await
            .unwrap();
        let result: DbResult = serde_json::from_value(result).unwrap();
        assert_eq!(result.affected_rows, Some(1));
    }

    #[tokio::test]
    async fn test_raw_query() {
        let mut plugin = create_test_db();

        // Create table using raw query
        let create_operation = DbOperation::Query {
            sql: "CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)".to_string(),
            params: None,
        };
        plugin
            .execute(Some(serde_json::to_value(create_operation).unwrap()), None)
            .await
            .unwrap();

        // Insert using raw query
        let insert_operation = DbOperation::Query {
            sql: "INSERT INTO test (value) VALUES (?)".to_string(),
            params: Some(vec![Value::String("test value".to_string())]),
        };
        let result = plugin
            .execute(Some(serde_json::to_value(insert_operation).unwrap()), None)
            .await
            .unwrap();
        let result: DbResult = serde_json::from_value(result).unwrap();
        assert_eq!(result.last_insert_id, Some(1));

        // Select using raw query
        let select_operation = DbOperation::Query {
            sql: "SELECT * FROM test WHERE id = ?".to_string(),
            params: Some(vec![Value::Number(1.into())]),
        };
        let result = plugin
            .execute(Some(serde_json::to_value(select_operation).unwrap()), None)
            .await
            .unwrap();
        let result: DbResult = serde_json::from_value(result).unwrap();
        let rows = result.rows.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["value"], Value::String("test value".to_string()));
    }
}
