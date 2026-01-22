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
        if item.name.to_lowercase() == "switch" {
            return Err(StoreError::InvalidData(
                "Name 'switch' is reserved for dynamic switching".to_string(),
            ));
        }
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
        if item.name.to_lowercase() == "switch" {
            return Err(StoreError::InvalidData(
                "Name 'switch' is reserved for dynamic switching".to_string(),
            ));
        }
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

    pub fn proxy_group_batch_update(
        &mut self,
        ids: Vec<i64>,
        prompt_injection: Option<String>,
        prompt_text: Option<String>,
        tool_filter: Option<String>,
        injection_position: Option<String>,
        injection_condition: Option<String>,
        prompt_replace: Option<Value>,
    ) -> Result<(), StoreError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        let tx = conn.transaction()?;

        for id in ids {
            // 1. Get current metadata to preserve other fields
            let mut metadata: Value = tx.query_row(
                &format!("SELECT metadata FROM {} WHERE id = ?1", PROXY_GROUP_TABLE),
                params![id],
                |row| {
                    let s: Option<String> = row.get(0)?;
                    Ok(s.and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or(serde_json::json!({})))
                },
            )?;

            // 2. Update metadata fields if provided
            if let Some(pos) = &injection_position {
                metadata["promptInjectionPosition"] = serde_json::json!(pos);
            }
            if let Some(cond) = &injection_condition {
                metadata["modelInjectionCondition"] = serde_json::json!(cond);
            }
            if let Some(replace) = &prompt_replace {
                metadata["promptReplace"] = replace.clone();
            }

            // 3. Build dynamic update query
            let mut updates = Vec::new();
            let mut values: Vec<rusqlite::types::Value> = Vec::new();

            if let Some(val) = &prompt_injection {
                updates.push("prompt_injection = ?");
                values.push(val.clone().into());
            }
            if let Some(val) = &prompt_text {
                updates.push("prompt_text = ?");
                values.push(val.clone().into());
            }
            if let Some(val) = &tool_filter {
                updates.push("tool_filter = ?");
                values.push(val.clone().into());
            }

            // Always update metadata as we merged it
            updates.push("metadata = ?");
            values.push(serde_json::to_string(&metadata).unwrap_or_default().into());

            if !updates.is_empty() {
                let sql = format!(
                    "UPDATE {} SET {} WHERE id = ?",
                    PROXY_GROUP_TABLE,
                    updates.join(", ")
                );
                values.push(id.into());
                tx.execute(&sql, rusqlite::params_from_iter(values))?;
            }
        }

        tx.commit()?;

        // Update cache
        if let Ok(pg) = Self::proxy_group_list(&conn) {
            self.config.set_proxy_groups(pg);
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
