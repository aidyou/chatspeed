use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Database operation configuration
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "operation")]
pub enum DbOperation {
    /// Execute raw SQL query with optional parameters
    #[serde(rename = "query")]
    Query {
        sql: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        params: Option<Vec<Value>>,
    },

    /// Insert data into table
    #[serde(rename = "insert")]
    Insert {
        table: String,
        data: Map<String, Value>,
    },

    /// Update data in table
    #[serde(rename = "update")]
    Update {
        table: String,
        data: Map<String, Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        params: Option<Vec<Value>>,
        where_clause: String,
    },

    /// Delete data from table
    #[serde(rename = "delete")]
    Delete {
        table: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        params: Option<Vec<Value>>,
        where_clause: String,
    },

    /// Select data from table
    #[serde(rename = "select")]
    Select {
        table: String,
        columns: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        params: Option<Vec<Value>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        where_clause: Option<String>,
    },

    /// Create table
    #[serde(rename = "create_table")]
    CreateTable {
        table: String,
        columns: Vec<ColumnDef>,
    },

    /// Drop table
    #[serde(rename = "drop_table")]
    DropTable {
        table: String,
    },
}

/// Column definition for table creation
#[derive(Debug, Serialize, Deserialize)]
pub struct ColumnDef {
    /// Column name
    pub name: String,
    /// Column type (TEXT, INTEGER, REAL, BLOB)
    pub type_name: String,
    /// Column constraints
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub constraints: Vec<String>,
}

/// Database operation result
#[derive(Debug, Serialize, Deserialize)]
pub struct DbResult {
    /// Number of affected rows for write operations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affected_rows: Option<u64>,
    /// Last inserted row id for insert operations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_insert_id: Option<i64>,
    /// Query results for select operations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<Vec<Value>>,
}
