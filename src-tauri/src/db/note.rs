use chrono::Utc;
use rusqlite::{params, OptionalExtension, Result as SqliteResult};
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use xxhash_rust::xxh32::xxh32;

use crate::db::error::StoreError;
use crate::db::main_store::MainStore;

/// Represents a note with its metadata.
#[derive(Debug, Serialize, Deserialize)]
pub struct Note {
    /// Unique identifier of the note.
    pub id: i64,
    /// Title of the note.
    pub title: String,
    /// Content of the note.
    pub content: String,
    /// Hash of the note content.
    #[serde(rename = "contentHash")]
    pub content_hash: String,
    /// Optional ID of the associated conversation.
    #[serde(rename = "conversationId")]
    pub conversation_id: Option<i64>,
    /// Optional ID of the associated message.
    #[serde(rename = "messageId")]
    pub message_id: Option<i64>,
    /// Timestamp when the note was created.
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    /// Timestamp when the note was last updated.
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
    // All note tags, separated by commas
    pub tags: Vec<String>,
    // Metadata associated with the note
    pub metadata: Option<serde_json::Value>,
}

/// Represents a tag with its metadata.
#[derive(Debug, Serialize, Deserialize)]
pub struct NoteTag {
    /// Unique identifier of the tag.
    pub id: i64,
    /// Name of the tag.
    pub name: String,
    /// Number of notes associated with the tag.
    #[serde(rename = "noteCount")]
    pub note_count: i64,
    /// Timestamp when the tag was created.
    #[serde(rename = "createdAt")]
    pub created_at: i64,
}

impl MainStore {
    pub(crate) async fn add_note_with_runtime(
        runtime: std::sync::Arc<crate::db::runtime::DbRuntime>,
        title: String,
        content: String,
        conversation_id: Option<i64>,
        message_id: Option<i64>,
        tags: Vec<String>,
        metadata: Option<serde_json::Value>,
    ) -> Result<i64, StoreError> {
        let content_hash = format!("{:x}", xxh32(content.as_bytes(), 0));
        let metadata_json = metadata
            .map(|value| serde_json::to_string(&value))
            .transpose()
            .map_err(|error| {
                StoreError::JsonError(
                    t!(
                        "db.json_serialize_failed_metadata",
                        error = error.to_string()
                    )
                    .to_string(),
                )
            })?;
        let now = Utc::now().timestamp();
        runtime
            .write(move |conn| {
                let exists = conn
                    .query_row(
                        "SELECT id FROM notes WHERE content_hash = ?1 AND (conversation_id = ?2 OR (?2 IS NULL AND conversation_id IS NULL)) AND (message_id = ?3 OR (?3 IS NULL AND message_id IS NULL)) AND deleted_at IS NULL LIMIT 1",
                        params![content_hash, conversation_id, message_id],
                        |_| Ok(true),
                    )
                    .optional()?;
                if exists.unwrap_or(false) {
                    return Err(StoreError::AlreadyExists(t!("chat.note_already_exists").into()));
                }
                let transaction = conn.transaction()?;
                transaction.execute(
                    "INSERT INTO notes (tags, title, content, content_hash, conversation_id, message_id, created_at, updated_at, metadata) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8)",
                    params![tags.join(","), title, content, content_hash, conversation_id, message_id, now, metadata_json],
                )?;
                let note_id = transaction.last_insert_rowid();
                for tag in tags {
                    transaction.execute(
                        "INSERT INTO note_tag_items (name, note_count, created_at) VALUES (?1, 1, ?2) ON CONFLICT(name) DO UPDATE SET note_count = note_count + 1",
                        params![tag, now],
                    )?;
                    let tag_id: i64 = transaction.query_row(
                        "SELECT id FROM note_tag_items WHERE name = ?1",
                        params![tag],
                        |row| row.get(0),
                    )?;
                    transaction.execute(
                        "INSERT INTO note_tag_relations (note_id, tag_id, created_at) VALUES (?1, ?2, ?3)",
                        params![note_id, tag_id, now],
                    )?;
                }
                transaction.commit()?;
                Ok(note_id)
            })
            .await
    }

    pub(crate) async fn delete_note_with_runtime(
        runtime: std::sync::Arc<crate::db::runtime::DbRuntime>,
        note_id: i64,
    ) -> Result<(), StoreError> {
        runtime
            .write(move |conn| {
                let transaction = conn.transaction()?;
                let tag_ids = {
                    let mut statement = transaction.prepare(
                        "SELECT tag_id FROM note_tag_relations WHERE note_id = ?1",
                    )?;
                    let rows = statement.query_map(params![note_id], |row| row.get::<_, i64>(0))?;
                    rows.collect::<Result<Vec<_>, _>>()?
                };
                for tag_id in tag_ids {
                    transaction.execute(
                        "UPDATE note_tag_items SET note_count = note_count - 1 WHERE id = ?1 AND note_count > 0",
                        params![tag_id],
                    )?;
                }
                transaction.execute(
                    "DELETE FROM note_tag_relations WHERE note_id = ?1",
                    params![note_id],
                )?;
                transaction.execute("DELETE FROM note_tag_items WHERE note_count = 0", [])?;
                transaction.execute("DELETE FROM notes WHERE id = ?1", params![note_id])?;
                transaction.commit()?;
                Ok(())
            })
            .await
    }

    /// Gets a note by its ID on a dedicated reader worker.
    pub(crate) async fn get_note_with_runtime(
        runtime: std::sync::Arc<crate::db::runtime::DbRuntime>,
        note_id: i64,
    ) -> Result<Note, StoreError> {
        runtime
            .read(move |conn| {
                let mut statement = conn.prepare("SELECT * FROM notes WHERE id = ?1")?;
                statement
                    .query_row(params![note_id], |row| {
                        let metadata_str: Option<String> = row.get("metadata")?;
                        let metadata =
                            metadata_str.and_then(|value| serde_json::from_str(&value).ok());
                        Ok(Note {
                            id: row.get("id")?,
                            tags: row
                                .get::<_, Option<String>>("tags")?
                                .unwrap_or_default()
                                .split(',')
                                .filter(|value| !value.is_empty())
                                .map(ToString::to_string)
                                .collect(),
                            title: row.get("title")?,
                            content: row.get("content")?,
                            content_hash: row.get("content_hash")?,
                            conversation_id: row.get("conversation_id")?,
                            message_id: row.get("message_id")?,
                            created_at: row.get("created_at")?,
                            updated_at: row.get("updated_at")?,
                            metadata,
                        })
                    })
                    .map_err(StoreError::from)
            })
            .await
    }

    pub(crate) async fn get_notes_with_runtime(
        runtime: std::sync::Arc<crate::db::runtime::DbRuntime>,
        tag_id: Option<i64>,
    ) -> Result<Vec<Note>, StoreError> {
        runtime
            .read(move |conn| {
                let sql = if tag_id.is_some() {
                    "SELECT n.* FROM notes n INNER JOIN note_tag_relations r ON n.id = r.note_id WHERE r.tag_id = ?1 AND n.deleted_at IS NULL ORDER BY n.created_at DESC"
                } else {
                    "SELECT * FROM notes WHERE deleted_at IS NULL ORDER BY created_at DESC"
                };
                let mut statement = conn.prepare(sql)?;
                let map_note = |row: &rusqlite::Row| -> SqliteResult<Note> {
                    let metadata_str: Option<String> = row.get("metadata")?;
                    Ok(Note {
                        id: row.get("id")?,
                        title: row.get("title")?,
                        content: row.get("content")?,
                        content_hash: row.get("content_hash")?,
                        conversation_id: row.get("conversation_id")?,
                        message_id: row.get("message_id")?,
                        created_at: row.get("created_at")?,
                        updated_at: row.get("updated_at")?,
                        tags: row.get::<_, Option<String>>("tags")?.unwrap_or_default().split(',').filter(|tag| !tag.is_empty()).map(ToString::to_string).collect(),
                        metadata: metadata_str.and_then(|value| serde_json::from_str(&value).ok()),
                    })
                };
                let rows = match tag_id {
                    Some(tag_id) => statement.query_map(params![tag_id], map_note)?,
                    None => statement.query_map([], map_note)?,
                };
                Ok(rows.collect::<SqliteResult<Vec<_>>>()?)
            })
            .await
    }

    /// Searches for notes based on a keyword in both title and content.
    ///
    /// # Arguments
    ///
    /// * `kw` - The keyword to search for. The search is case-insensitive and uses SQL LIKE with wildcards.
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing:
    /// - `Ok(Vec<Note>)`: A vector of notes that match the search criteria, ordered by last updated time (newest first).
    /// - `Err(StoreError)`: If there was an error executing the database query.
    ///
    /// # Example
    /// ```no_run
    /// use crate::db::MainStore;
    /// let store = MainStore::new()?;
    /// let notes = store.search_notes("keyword")?;
    /// ```
    pub(crate) async fn search_notes_with_runtime(
        runtime: std::sync::Arc<crate::db::runtime::DbRuntime>,
        keyword: String,
    ) -> Result<Vec<Note>, StoreError> {
        runtime
            .read(move |conn| {
                let mut statement = conn.prepare(
                    "SELECT n.* FROM notes n WHERE n.title LIKE ?1 ORDER BY n.updated_at DESC",
                )?;
                let pattern = format!("%{}%", keyword);
                let rows = statement.query_map(params![pattern], |row| {
                    let metadata_text: Option<String> = row.get("metadata")?;
                    Ok(Note {
                        id: row.get("id")?,
                        title: row.get("title")?,
                        content: row.get("content")?,
                        content_hash: row.get("content_hash")?,
                        conversation_id: row.get("conversation_id")?,
                        message_id: row.get("message_id")?,
                        created_at: row.get("created_at")?,
                        updated_at: row.get("updated_at")?,
                        tags: row
                            .get::<_, Option<String>>("tags")?
                            .unwrap_or_default()
                            .split(',')
                            .filter(|tag| !tag.is_empty())
                            .map(ToString::to_string)
                            .collect(),
                        metadata: metadata_text.and_then(|value| serde_json::from_str(&value).ok()),
                    })
                })?;
                Ok(rows.collect::<SqliteResult<Vec<_>>>()?)
            })
            .await
    }

    pub(crate) async fn get_tags_with_runtime(
        runtime: std::sync::Arc<crate::db::runtime::DbRuntime>,
    ) -> Result<Vec<NoteTag>, StoreError> {
        runtime
            .read(|conn| {
                let mut statement =
                    conn.prepare("SELECT * FROM note_tag_items ORDER BY name ASC")?;
                let rows = statement.query_map([], |row| {
                    Ok(NoteTag {
                        id: row.get("id")?,
                        name: row.get("name")?,
                        note_count: row.get("note_count")?,
                        created_at: row.get("created_at")?,
                    })
                })?;
                Ok(rows.collect::<SqliteResult<Vec<_>>>()?)
            })
            .await
    }
}
