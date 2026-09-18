//! ChatHub (web chat entry) persistence.
//!
//! ChatHub rows only store entry metadata (name, logo, url and order). Page
//! content, cookies and browsing data stay inside the platform webview and are
//! never persisted here. ChatHub deliberately lives outside the `config` table
//! and outside `MainStore`'s `ConfigCache` because it is independent state that
//! must not be bundled into generic settings.
use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::db::{MainStore, StoreError};

pub const CHAT_HUB_TABLE: &str = "chat_hubs";

/// Maximum accepted length for a ChatHub display name.
const MAX_NAME_LENGTH: usize = 200;
/// Maximum accepted length for a ChatHub url or logo url.
const MAX_URL_LENGTH: usize = 2048;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChatHub {
    pub id: i64,
    pub name: String,
    pub logo: String,
    pub url: String,
    pub sort_index: i64,
    pub is_default: bool,
}

impl ChatHub {
    fn from_row(row: &Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            logo: row.get("logo")?,
            url: row.get("url")?,
            sort_index: row.get("sort_index")?,
            is_default: row.get("is_default")?,
        })
    }
}

/// Validates and normalizes a ChatHub navigation url.
///
/// Only `http` and `https` urls with a host are accepted, so an entry can never
/// point the external webview at an application, file or data url.
pub fn parse_chat_hub_url(raw: &str) -> Result<Url, StoreError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(StoreError::InvalidData(
            "ChatHub url must not be empty".to_string(),
        ));
    }
    if trimmed.len() > MAX_URL_LENGTH {
        return Err(StoreError::InvalidData(format!(
            "ChatHub url must not exceed {} characters",
            MAX_URL_LENGTH
        )));
    }

    let parsed = Url::parse(trimmed)
        .map_err(|error| StoreError::InvalidData(format!("Invalid ChatHub url: {}", error)))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(StoreError::InvalidData(
            "ChatHub url must be an absolute http or https url".to_string(),
        ));
    }

    Ok(parsed)
}

/// Validates and normalizes a ChatHub navigation url into its canonical string form.
pub fn normalize_chat_hub_url(raw: &str) -> Result<String, StoreError> {
    Ok(parse_chat_hub_url(raw)?.to_string())
}

/// Validates and normalizes an optional ChatHub logo url.
///
/// An empty logo is valid and means "fall back to the letter avatar". A non
/// empty logo must be an absolute `http`/`https` url so it can only ever be used
/// as a remote image source.
pub fn normalize_chat_hub_logo(raw: &str) -> Result<String, StoreError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.len() > MAX_URL_LENGTH {
        return Err(StoreError::InvalidData(format!(
            "ChatHub logo must not exceed {} characters",
            MAX_URL_LENGTH
        )));
    }

    let parsed = Url::parse(trimmed)
        .map_err(|error| StoreError::InvalidData(format!("Invalid ChatHub logo url: {}", error)))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(StoreError::InvalidData(
            "ChatHub logo must be an absolute http or https url".to_string(),
        ));
    }

    Ok(parsed.to_string())
}

/// Validates and normalizes a ChatHub display name.
pub fn normalize_chat_hub_name(raw: &str) -> Result<String, StoreError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(StoreError::InvalidData(
            "ChatHub name must not be empty".to_string(),
        ));
    }
    if trimmed.chars().count() > MAX_NAME_LENGTH {
        return Err(StoreError::InvalidData(format!(
            "ChatHub name must not exceed {} characters",
            MAX_NAME_LENGTH
        )));
    }

    Ok(trimmed.to_string())
}

impl MainStore {
    pub(crate) fn chat_hub_list(conn: &Connection) -> Result<Vec<ChatHub>, StoreError> {
        let mut stmt = conn.prepare(&format!(
            "SELECT * FROM {} ORDER BY sort_index ASC, id ASC",
            CHAT_HUB_TABLE
        ))?;
        let rows = stmt.query_map([], ChatHub::from_row)?;

        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }

        Ok(items)
    }

    fn chat_hub_by_id(conn: &Connection, id: i64) -> Result<ChatHub, StoreError> {
        conn.query_row(
            &format!("SELECT * FROM {} WHERE id = ?1", CHAT_HUB_TABLE),
            params![id],
            ChatHub::from_row,
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("ChatHub entry {} was not found", id)))
    }

    /// Returns every ChatHub entry ordered by `sort_index`.
    pub fn get_all_chat_hubs(&self) -> Result<Vec<ChatHub>, StoreError> {
        self.db_runtime()?
            .read_blocking(|conn| Self::chat_hub_list(conn))
    }

    /// Appends a new ChatHub entry at the end of the list.
    pub fn add_chat_hub(&self, name: &str, logo: &str, url: &str) -> Result<ChatHub, StoreError> {
        let name = normalize_chat_hub_name(name)?;
        let logo = normalize_chat_hub_logo(logo)?;
        let url = normalize_chat_hub_url(url)?;

        self.db_runtime()?.write_blocking(move |conn| {
            let next_sort_index: i64 = conn.query_row(
                &format!("SELECT COALESCE(MAX(sort_index), -1) + 1 FROM {}", CHAT_HUB_TABLE),
                [],
                |row| row.get(0),
            )?;
            conn.execute(
                &format!(
                    "INSERT INTO {} (name, logo, url, sort_index, is_default) VALUES (?1, ?2, ?3, ?4, 0)",
                    CHAT_HUB_TABLE
                ),
                params![name, logo, url, next_sort_index],
            )?;
            Self::chat_hub_by_id(conn, conn.last_insert_rowid())
        })
    }

    /// Updates the editable fields of an existing ChatHub entry.
    ///
    /// Presets can be updated and deleted like any other entry; `is_default` is
    /// only a marker of where the entry came from.
    pub fn update_chat_hub(
        &self,
        id: i64,
        name: &str,
        logo: &str,
        url: &str,
    ) -> Result<ChatHub, StoreError> {
        let name = normalize_chat_hub_name(name)?;
        let logo = normalize_chat_hub_logo(logo)?;
        let url = normalize_chat_hub_url(url)?;

        self.db_runtime()?.write_blocking(move |conn| {
            let affected = conn.execute(
                &format!(
                    "UPDATE {} SET name = ?1, logo = ?2, url = ?3 WHERE id = ?4",
                    CHAT_HUB_TABLE
                ),
                params![name, logo, url, id],
            )?;
            if affected == 0 {
                return Err(StoreError::NotFound(format!(
                    "ChatHub entry {} was not found",
                    id
                )));
            }
            Self::chat_hub_by_id(conn, id)
        })
    }

    /// Deletes a ChatHub entry permanently, including presets.
    pub fn delete_chat_hub(&self, id: i64) -> Result<(), StoreError> {
        self.db_runtime()?.write_blocking(move |conn| {
            conn.execute(
                &format!("DELETE FROM {} WHERE id = ?1", CHAT_HUB_TABLE),
                params![id],
            )?;
            Ok(())
        })
    }

    /// Persists the complete ChatHub order in a single transaction.
    ///
    /// The submitted ids must be an exact permutation of the stored ids so a
    /// partial or duplicated list can never leave rows with stale indices.
    pub fn update_chat_hub_order(&self, hub_ids: Vec<i64>) -> Result<(), StoreError> {
        let mut unique_ids = HashSet::with_capacity(hub_ids.len());
        for id in &hub_ids {
            if !unique_ids.insert(*id) {
                return Err(StoreError::InvalidData(format!(
                    "Duplicate ChatHub id {} in the submitted order",
                    id
                )));
            }
        }

        self.db_runtime()?.write_blocking(move |conn| {
            let existing = Self::chat_hub_list(conn)?;
            if existing.len() != hub_ids.len()
                || existing.iter().any(|hub| !unique_ids.contains(&hub.id))
            {
                return Err(StoreError::InvalidData(
                    "Submitted ChatHub order must contain every existing entry exactly once"
                        .to_string(),
                ));
            }

            let tx = conn.transaction()?;
            for (index, id) in hub_ids.iter().enumerate() {
                tx.execute(
                    &format!(
                        "UPDATE {} SET sort_index = ?1 WHERE id = ?2",
                        CHAT_HUB_TABLE
                    ),
                    params![index as i64, id],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_test_store() -> (tempfile::TempDir, MainStore) {
        let dir = tempdir().expect("failed to create temp dir");
        let db_path = dir.path().join("chat_hub_test.db");
        let store = MainStore::new(db_path).expect("failed to create MainStore");
        (dir, store)
    }

    #[test]
    fn list_returns_seeded_presets_sorted_by_index() {
        let (_dir, store) = create_test_store();
        let hubs = store.get_all_chat_hubs().expect("failed to list chat hubs");

        assert!(
            !hubs.is_empty(),
            "fresh database should seed preset entries"
        );
        assert!(
            hubs.iter().all(|hub| hub.is_default),
            "seeded entries should be marked as presets"
        );
        let mut sorted = hubs.clone();
        sorted.sort_by_key(|hub| (hub.sort_index, hub.id));
        assert_eq!(hubs, sorted, "entries must be returned in sort order");
    }

    #[test]
    fn add_appends_entry_and_lets_sqlite_assign_the_id() {
        let (_dir, store) = create_test_store();
        let before = store.get_all_chat_hubs().expect("failed to list chat hubs");

        let created = store
            .add_chat_hub("Custom", "", "https://example.com/chat")
            .expect("failed to add chat hub");

        assert!(created.id > 0, "sqlite should assign a positive id");
        assert!(!created.is_default, "new entries are not presets");
        assert_eq!(created.sort_index, before.len() as i64);
        assert_eq!(created.url, "https://example.com/chat");

        let after = store.get_all_chat_hubs().expect("failed to list chat hubs");
        assert_eq!(after.len(), before.len() + 1);
        assert_eq!(after.last().map(|hub| hub.id), Some(created.id));
    }

    #[test]
    fn update_persists_fields_and_keeps_preset_deletable() {
        let (_dir, store) = create_test_store();
        let preset = store
            .get_all_chat_hubs()
            .expect("failed to list chat hubs")
            .into_iter()
            .next()
            .expect("expected a preset entry");

        let updated = store
            .update_chat_hub(
                preset.id,
                "Renamed",
                "https://cdn.example.com/logo.png",
                "https://example.org/",
            )
            .expect("failed to update chat hub");
        assert_eq!(updated.name, "Renamed");
        assert_eq!(updated.logo, "https://cdn.example.com/logo.png");
        assert_eq!(updated.url, "https://example.org/");
        assert!(
            updated.is_default,
            "updating a preset keeps its origin marker"
        );

        store
            .delete_chat_hub(preset.id)
            .expect("preset entries must be deletable");
        let remaining = store.get_all_chat_hubs().expect("failed to list chat hubs");
        assert!(
            remaining.iter().all(|hub| hub.id != preset.id),
            "deleted preset must not come back"
        );
    }

    #[test]
    fn rejects_urls_that_are_not_http_or_https() {
        for raw in [
            "",
            "   ",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,<h1>hi</h1>",
            "tauri://localhost/index.html",
            "not a url",
        ] {
            assert!(
                normalize_chat_hub_url(raw).is_err(),
                "url {:?} must be rejected",
                raw
            );
        }

        assert_eq!(
            normalize_chat_hub_url("  https://example.com/a  ").expect("valid url"),
            "https://example.com/a"
        );
    }

    #[test]
    fn rejects_logo_urls_with_unsupported_schemes_but_allows_empty() {
        assert_eq!(normalize_chat_hub_logo("").expect("empty logo"), "");
        assert_eq!(normalize_chat_hub_logo("  ").expect("blank logo"), "");
        assert!(normalize_chat_hub_logo("data:image/png;base64,AAAA").is_err());
        assert!(normalize_chat_hub_logo("file:///tmp/logo.png").is_err());
        assert_eq!(
            normalize_chat_hub_logo("https://www.google.com/s2/favicons?sz=64&domain_url=a")
                .expect("valid logo"),
            "https://www.google.com/s2/favicons?sz=64&domain_url=a"
        );
    }

    #[test]
    fn rejects_empty_names_and_unknown_ids() {
        let (_dir, store) = create_test_store();
        assert!(store
            .add_chat_hub("  ", "", "https://example.com/")
            .is_err());
        assert!(store
            .update_chat_hub(999_999, "Name", "", "https://example.com/")
            .is_err());
        assert!(store
            .add_chat_hub("Name", "", "ftp://example.com/")
            .is_err());
    }

    #[test]
    fn reorder_applies_the_submitted_permutation_in_one_transaction() {
        let (_dir, store) = create_test_store();
        let hubs = store.get_all_chat_hubs().expect("failed to list chat hubs");
        let mut reversed: Vec<i64> = hubs.iter().map(|hub| hub.id).collect();
        reversed.reverse();

        store
            .update_chat_hub_order(reversed.clone())
            .expect("failed to reorder chat hubs");

        let reordered = store.get_all_chat_hubs().expect("failed to list chat hubs");
        let ids: Vec<i64> = reordered.iter().map(|hub| hub.id).collect();
        assert_eq!(ids, reversed);
        let indices: Vec<i64> = reordered.iter().map(|hub| hub.sort_index).collect();
        assert_eq!(indices, (0..hubs.len() as i64).collect::<Vec<_>>());
    }

    #[test]
    fn reorder_rejects_partial_duplicate_and_unknown_ids_without_writing() {
        let (_dir, store) = create_test_store();
        let hubs = store.get_all_chat_hubs().expect("failed to list chat hubs");
        let ids: Vec<i64> = hubs.iter().map(|hub| hub.id).collect();

        let mut duplicate = ids.clone();
        duplicate[0] = duplicate[1];
        assert!(store.update_chat_hub_order(duplicate).is_err());

        assert!(store
            .update_chat_hub_order(ids[..ids.len() - 1].to_vec())
            .is_err());

        let mut unknown = ids.clone();
        unknown[0] = 999_999;
        assert!(store.update_chat_hub_order(unknown).is_err());

        assert!(store.update_chat_hub_order(Vec::new()).is_err());

        let unchanged = store.get_all_chat_hubs().expect("failed to list chat hubs");
        assert_eq!(
            unchanged, hubs,
            "a rejected reorder must not change stored data"
        );
    }
}
