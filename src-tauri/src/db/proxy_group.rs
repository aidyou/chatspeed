//! Proxy group manager
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::db::MainStore;

use super::StoreError;

pub const PROXY_GROUP_TABLE: &str = "proxy_group";

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ProxyGroup {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub prompt_injection: String,
    pub prompt_text: String,
    pub tool_filter: String,
    pub disabled: bool,
}

impl MainStore {
    pub fn proxy_group_list(&self) -> Result<Vec<ProxyGroup>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::FailedToLockMainStore(e.to_string()))?;
        let mut stmt = conn.prepare(&format!(
            "SELECT id, name, description, prompt_injection, prompt_text, tool_filter, disabled FROM {} ORDER BY id DESC",
            PROXY_GROUP_TABLE
        ))?;
        let rows = stmt.query_map([], |row| {
            Ok(ProxyGroup {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                prompt_injection: row.get(3)?,
                prompt_text: row.get(4)?,
                tool_filter: row.get(5)?,
                disabled: row.get(6)?,
            })
        })?;

        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }

        Ok(items)
    }

    pub fn proxy_group_add(&self, item: &ProxyGroup) -> Result<i64, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::FailedToLockMainStore(e.to_string()))?;
        let mut stmt = conn.prepare(&format!(
            "INSERT INTO {} (name, description, prompt_injection, prompt_text, tool_filter, disabled) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            PROXY_GROUP_TABLE
        ))?;
        let id = stmt.insert(params![
            item.name,
            item.description,
            item.prompt_injection,
            item.prompt_text,
            item.tool_filter,
            item.disabled
        ])?;
        Ok(id)
    }

    pub fn proxy_group_update(&self, item: &ProxyGroup) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::FailedToLockMainStore(e.to_string()))?;
        let mut stmt = conn.prepare(&format!(
            "UPDATE {} SET name = ?1, description = ?2, prompt_injection = ?3, prompt_text = ?4, tool_filter = ?5, disabled = ?6 WHERE id = ?7",
            PROXY_GROUP_TABLE
        ))?;
        stmt.execute(params![
            item.name,
            item.description,
            item.prompt_injection,
            item.prompt_text,
            item.tool_filter,
            item.disabled,
            item.id
        ])?;
        Ok(())
    }

    pub fn proxy_group_delete(&self, id: i64) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::FailedToLockMainStore(e.to_string()))?;
        let mut stmt = conn.prepare(&format!("DELETE FROM {} WHERE id = ?1", PROXY_GROUP_TABLE))?;
        stmt.execute(params![id])?;
        Ok(())
    }
}
