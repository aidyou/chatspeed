//! Proxy group manager
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::MainStore;

use super::StoreError;

pub const PROXY_GROUP_TABLE: &str = "proxy_group";

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProxyGroup {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub prompt_injection: String,
    pub prompt_text: String,
    pub tool_filter: String,
    pub temperature: Option<f32>,
    pub metadata: Option<Value>,
    pub disabled: bool,
}

impl MainStore {
    pub(crate) fn proxy_group_list(conn: &Connection) -> Result<Vec<ProxyGroup>, StoreError> {
        let mut stmt = conn.prepare(&format!(
            "SELECT * FROM {} ORDER BY id DESC",
            PROXY_GROUP_TABLE
        ))?;
        let rows = stmt.query_map([], |row| {
            let metadata_str: Option<String> = row.get("metadata")?; // metadata is JSON string
            let metadata = metadata_str.and_then(|s| {
                serde_json::from_str(&s)
                    .map_err(|e| {
                        log::warn!(
                            "Failed to parse metadata JSON for AI Model (id: {:?}): {}, error: {}",
                            row.get::<_, Option<i64>>("id").unwrap_or_default(),
                            s,
                            e
                        );
                        e
                    })
                    .ok()
            });
            Ok(ProxyGroup {
                id: row.get("id")?,
                name: row.get("name")?,
                description: row.get("description")?,
                prompt_injection: row.get("prompt_injection")?,
                prompt_text: row.get("prompt_text")?,
                tool_filter: row.get("tool_filter")?,
                temperature: Some(row.get("temperature").unwrap_or(1.0)),
                metadata: metadata,
                disabled: row.get("disabled")?,
            })
        })?;

        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }

        Ok(items)
    }

    pub fn proxy_group_add(&mut self, item: &ProxyGroup) -> Result<i64, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let mut stmt = conn.prepare(&format!(
            "INSERT INTO {} (name, description, prompt_injection, prompt_text, tool_filter, temperature, metadata, disabled) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            PROXY_GROUP_TABLE
        ))?;
        let id = stmt.insert(params![
            item.name,
            item.description,
            item.prompt_injection,
            item.prompt_text,
            item.tool_filter,
            item.temperature.unwrap_or(1.0),
            item.metadata
                .as_ref()
                .map(|m| serde_json::to_string(&m).unwrap_or_default()),
            item.disabled
        ])?;
        if id > 0 {
            if let Ok(pg) = Self::proxy_group_list(&conn) {
                self.config.set_proxy_groups(pg);
            }
        }
        Ok(id)
    }

    pub fn proxy_group_update(&mut self, item: &ProxyGroup) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let mut stmt = conn.prepare(&format!(
            "UPDATE {} SET name = ?1, description = ?2, prompt_injection = ?3, prompt_text = ?4, tool_filter = ?5, temperature = ?6, metadata = ?7, disabled = ?8 WHERE id = ?9",
            PROXY_GROUP_TABLE
        ))?;
        let changed = stmt.execute(params![
            item.name,
            item.description,
            item.prompt_injection,
            item.prompt_text,
            item.tool_filter,
            item.temperature.unwrap_or(1.0),
            item.metadata
                .as_ref()
                .map(|m| serde_json::to_string(&m).unwrap_or_default()),
            item.disabled,
            item.id
        ])?;

        if changed > 0 {
            if let Ok(pg) = Self::proxy_group_list(&conn) {
                self.config.set_proxy_groups(pg);
            }
        }
        Ok(())
    }

    pub fn proxy_group_delete(&mut self, id: i64) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let mut stmt = conn.prepare(&format!("DELETE FROM {} WHERE id = ?1", PROXY_GROUP_TABLE))?;
        let changed = stmt.execute(params![id])?;

        if changed > 0 {
            if let Ok(pg) = Self::proxy_group_list(&conn) {
                self.config.set_proxy_groups(pg);
            }
        }

        Ok(())
    }
}
