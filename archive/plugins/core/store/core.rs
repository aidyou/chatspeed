//! Core SQLite Database Operations Module
//!
//! This module provides core functionality for SQLite database operations, including:
//! - Database connection management
//! - SQL statement execution
//! - Parameter binding and type conversion
//! - Query result processing
//!
//! # Examples
//!
//! ## Basic Query Operations
//! ```rust
//! use crate::plugins::core::store::Store;
//!
//! let store = Store::new("test.db")?;
//!
//! // Execute a query
//! let result = store.execute_query(
//!     "SELECT * FROM users WHERE age > ?",
//!     Some(vec![serde_json::Value::Number(18.into())]),
//! )?;
//!
//! if let Some(rows) = result.rows {
//!     for row in rows {
//!         println!("User: {}", row);
//!     }
//! }
//! ```
//!
//! ## Transaction Operations
//! ```rust
//! use crate::plugins::core::store::Store;
//! use crate::plugins::core::store::types::DbOperation;
//!
//! let store = Store::new("test.db")?;
//!
//! // Create table
//! store.execute_query(
//!     "CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT)",
//!     None,
//! )?;
//!
//! // Insert data
//! let mut data = serde_json::Map::new();
//! data.insert("name".to_string(), serde_json::Value::String("John".to_string()));
//!
//! let operation = DbOperation::Insert {
//!     table: "users".to_string(),
//!     data,
//! };
//!
//! let result = store.handle_operation(operation)?;
//! println!("Inserted row ID: {}", result.last_insert_id.unwrap());
//! ```

use super::types::{DbOperation, DbResult};
use crate::plugins::PluginError;
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{types::ToSql, Connection, Row};
use rust_i18n::t;
use serde_json::{Map, Number, Value};
use std::sync::Mutex;

/// SQLite store implementation
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Creates a new store instance
    pub fn new(db_path: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let conn = Connection::open(db_path).map_err(|e| {
            Box::new(PluginError::RuntimeError(
                t!("store.connection_failed", error = e.to_string()).to_string(),
            )) as Box<dyn std::error::Error + Send + Sync>
        })?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Converts a row to JSON value
    fn row_to_json(row: &Row) -> rusqlite::Result<Value> {
        let mut map = Map::new();
        let columns = row.as_ref().column_names();

        for (i, column_name) in columns.iter().enumerate() {
            let value = match row.get_ref(i)? {
                rusqlite::types::ValueRef::Null => Value::Null,
                rusqlite::types::ValueRef::Integer(i) => Value::Number(i.into()),
                rusqlite::types::ValueRef::Real(f) => match Number::from_f64(f) {
                    Some(n) => Value::Number(n),
                    None => Value::Number(Number::from_f64(0.0).ok_or_else(|| {
                        rusqlite::Error::FromSqlConversionFailure(
                            i,
                            rusqlite::types::Type::Real,
                            Box::new(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "Failed to convert f64 to Number",
                            )),
                        )
                    })?),
                },
                rusqlite::types::ValueRef::Text(s) => {
                    Value::String(String::from_utf8_lossy(s).into())
                }
                rusqlite::types::ValueRef::Blob(b) => Value::String(STANDARD.encode(b)),
            };
            map.insert(column_name.to_string(), value);
        }

        Ok(Value::Object(map))
    }

    /// Converts JSON value to SQL parameter
    fn value_to_sql(value: &Value) -> Box<dyn ToSql> {
        match value {
            Value::Null => Box::new(None::<String>),
            Value::Bool(b) => Box::new(*b),
            Value::Number(n) => {
                if n.is_i64() {
                    Box::new(n.as_i64().unwrap_or(0))
                } else {
                    Box::new(n.as_f64().unwrap_or(0.0))
                }
            }
            Value::String(s) => Box::new(s.clone()),
            _ => Box::new(value.to_string()),
        }
    }

    /// Executes raw SQL query
    pub fn execute_query(
        &self,
        sql: &str,
        params: Option<Vec<Value>>,
    ) -> Result<DbResult, Box<dyn std::error::Error + Send + Sync>> {
        let conn = self.conn.lock().map_err(|e| {
            Box::new(PluginError::RuntimeError(
                t!("store.lock_failed", error = e.to_string()).to_string(),
            )) as Box<dyn std::error::Error + Send + Sync>
        })?;

        let mut stmt = conn.prepare(sql).map_err(|e| {
            Box::new(PluginError::RuntimeError(
                t!("store.prepare_failed", error = e.to_string()).to_string(),
            )) as Box<dyn std::error::Error + Send + Sync>
        })?;

        let params: Vec<Box<dyn ToSql>> = params
            .unwrap_or_default()
            .iter()
            .map(Self::value_to_sql)
            .collect();

        let param_refs: Vec<&dyn ToSql> = params.iter().map(|p| p.as_ref()).collect();

        let result = if sql.trim().to_lowercase().starts_with("select") {
            let rows: rusqlite::Result<Vec<Value>> = stmt
                .query(param_refs.as_slice())?
                .mapped(Self::row_to_json)
                .collect();

            Ok::<DbResult, Box<dyn std::error::Error + Send + Sync>>(DbResult {
                rows: Some(rows?),
                affected_rows: None,
                last_insert_id: None,
            })
        } else {
            let affected = stmt.execute(param_refs.as_slice()).map_err(|e| {
                Box::new(PluginError::RuntimeError(
                    t!("store.execute_failed", error = e.to_string()).to_string(),
                )) as Box<dyn std::error::Error + Send + Sync>
            })?;

            Ok::<DbResult, Box<dyn std::error::Error + Send + Sync>>(DbResult {
                rows: None,
                affected_rows: Some(affected as u64),
                last_insert_id: Some(conn.last_insert_rowid()),
            })
        }?;

        Ok(result)
    }

    /// Handles database operations
    pub fn handle_operation(
        &self,
        operation: DbOperation,
    ) -> Result<DbResult, Box<dyn std::error::Error + Send + Sync>> {
        match operation {
            DbOperation::Query { sql, params } => self.execute_query(&sql, params),

            DbOperation::Insert { table, data } => {
                let columns: Vec<String> = data.keys().cloned().collect();
                let values: Vec<Value> = data.values().cloned().collect();
                let placeholders = vec!["?"; values.len()].join(", ");

                let sql = format!(
                    "INSERT INTO {} ({}) VALUES ({})",
                    table,
                    columns.join(", "),
                    placeholders
                );

                self.execute_query(&sql, Some(values))
            }

            DbOperation::Update {
                table,
                data,
                where_clause,
                params,
            } => {
                let set_columns: Vec<String> = data.keys().map(|k| format!("{} = ?", k)).collect();
                let mut values: Vec<Value> = data.values().cloned().collect();

                if let Some(mut p) = params {
                    values.append(&mut p);
                }

                let sql = format!(
                    "UPDATE {} SET {} WHERE {}",
                    table,
                    set_columns.join(", "),
                    where_clause
                );

                self.execute_query(&sql, Some(values))
            }

            DbOperation::Delete {
                table,
                where_clause,
                params,
            } => {
                let sql = format!("DELETE FROM {} WHERE {}", table, where_clause);
                self.execute_query(&sql, params)
            }

            DbOperation::Select {
                table,
                columns,
                where_clause,
                params,
            } => {
                let columns = if columns.is_empty() {
                    "*".to_string()
                } else {
                    columns.join(", ")
                };

                let where_clause =
                    where_clause.map_or_else(String::new, |w| format!(" WHERE {}", w));
                let sql = format!("SELECT {} FROM {}{}", columns, table, where_clause);

                self.execute_query(&sql, params)
            }

            DbOperation::CreateTable { table, columns } => {
                let columns: Vec<String> = columns
                    .iter()
                    .map(|col| {
                        let constraints = if col.constraints.is_empty() {
                            String::new()
                        } else {
                            format!(" {}", col.constraints.join(" "))
                        };
                        format!("{} {}{}", col.name, col.type_name, constraints)
                    })
                    .collect();

                let sql = format!(
                    "CREATE TABLE IF NOT EXISTS {} ({})",
                    table,
                    columns.join(", ")
                );

                self.execute_query(&sql, None)
            }

            DbOperation::DropTable { table } => {
                let sql = format!("DROP TABLE IF EXISTS {}", table);
                self.execute_query(&sql, None)
            }
        }
    }
}
