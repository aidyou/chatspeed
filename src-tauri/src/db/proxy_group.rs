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

    pub fn proxy_group_add(&self, item: &ProxyGroup) -> Result<i64, StoreError> {
        if item.name.to_lowercase() == "switch" {
            return Err(StoreError::InvalidData(
                "Name 'switch' is reserved for dynamic switching".to_string(),
            ));
        }
        let _config_update_guard = self.config_update_lock.lock();
        let item = item.clone();
        let (id, groups) = self.db_runtime()?.write_blocking(move |conn| {
            let metadata = item
                .metadata
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|error| StoreError::JsonError(error.to_string()))?;
            conn.execute(
                &format!(
                    "INSERT INTO {} (name, description, prompt_injection, prompt_text, tool_filter, temperature, metadata, disabled) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    PROXY_GROUP_TABLE
                ),
                params![
                    item.name,
                    item.description,
                    item.prompt_injection,
                    item.prompt_text,
                    item.tool_filter,
                    item.temperature.unwrap_or(1.0),
                    metadata,
                    item.disabled
                ],
            )?;
            Ok((conn.last_insert_rowid(), Self::proxy_group_list(conn)?))
        })?;
        self.config.set_proxy_groups(groups);
        Ok(id)
    }

    pub fn proxy_group_update(&self, item: &ProxyGroup) -> Result<(), StoreError> {
        if item.name.to_lowercase() == "switch" {
            return Err(StoreError::InvalidData(
                "Name 'switch' is reserved for dynamic switching".to_string(),
            ));
        }
        let _config_update_guard = self.config_update_lock.lock();
        let item = item.clone();
        let groups = self.db_runtime()?.write_blocking(move |conn| {
            let metadata = item
                .metadata
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|error| StoreError::JsonError(error.to_string()))?;
            conn.execute(
                &format!(
                    "UPDATE {} SET name = ?1, description = ?2, prompt_injection = ?3, prompt_text = ?4, tool_filter = ?5, temperature = ?6, metadata = ?7, disabled = ?8 WHERE id = ?9",
                    PROXY_GROUP_TABLE
                ),
                params![
                    item.name,
                    item.description,
                    item.prompt_injection,
                    item.prompt_text,
                    item.tool_filter,
                    item.temperature.unwrap_or(1.0),
                    metadata,
                    item.disabled,
                    item.id
                ],
            )?;
            Self::proxy_group_list(conn)
        })?;
        self.config.set_proxy_groups(groups);
        Ok(())
    }

    pub fn proxy_group_batch_update(
        &self,
        ids: Vec<i64>,
        prompt_injection: Option<String>,
        prompt_text: Option<String>,
        tool_filter: Option<String>,
        injection_position: Option<String>,
        injection_condition: Option<String>,
        prompt_replace: Option<Value>,
    ) -> Result<(), StoreError> {
        let _config_update_guard = self.config_update_lock.lock();
        let groups = self.db_runtime()?.write_blocking(move |conn| {
            let tx = conn.transaction()?;

            for id in ids {
                let mut metadata: Value = tx.query_row(
                    &format!("SELECT metadata FROM {} WHERE id = ?1", PROXY_GROUP_TABLE),
                    params![id],
                    |row| {
                        let value: Option<String> = row.get(0)?;
                        Ok(value
                            .and_then(|value| serde_json::from_str(&value).ok())
                            .unwrap_or_else(|| serde_json::json!({})))
                    },
                )?;

                if let Some(position) = &injection_position {
                    metadata["promptInjectionPosition"] = serde_json::json!(position);
                }
                if let Some(condition) = &injection_condition {
                    metadata["modelInjectionCondition"] = serde_json::json!(condition);
                }
                if let Some(replace) = &prompt_replace {
                    metadata["promptReplace"] = replace.clone();
                }

                let mut updates = Vec::new();
                let mut values: Vec<rusqlite::types::Value> = Vec::new();
                if let Some(value) = &prompt_injection {
                    updates.push("prompt_injection = ?");
                    values.push(value.clone().into());
                }
                if let Some(value) = &prompt_text {
                    updates.push("prompt_text = ?");
                    values.push(value.clone().into());
                }
                if let Some(value) = &tool_filter {
                    updates.push("tool_filter = ?");
                    values.push(value.clone().into());
                }
                updates.push("metadata = ?");
                values.push(serde_json::to_string(&metadata)?.into());
                values.push(id.into());
                tx.execute(
                    &format!(
                        "UPDATE {} SET {} WHERE id = ?",
                        PROXY_GROUP_TABLE,
                        updates.join(", ")
                    ),
                    rusqlite::params_from_iter(values),
                )?;
            }

            tx.commit()?;
            Self::proxy_group_list(conn)
        })?;
        self.config.set_proxy_groups(groups);
        Ok(())
    }

    pub fn proxy_group_delete(&self, id: i64) -> Result<(), StoreError> {
        let _config_update_guard = self.config_update_lock.lock();
        let groups = self.db_runtime()?.write_blocking(move |conn| {
            conn.execute(
                &format!("DELETE FROM {} WHERE id = ?1", PROXY_GROUP_TABLE),
                params![id],
            )?;
            Self::proxy_group_list(conn)
        })?;
        self.config.set_proxy_groups(groups);
        Ok(())
    }
}
