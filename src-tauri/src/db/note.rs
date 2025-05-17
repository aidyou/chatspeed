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
    /// Creates a new note with the given title, content, and optional tags.
    ///
    /// # Arguments
    ///
    /// * `title` - The title of the note.
    /// * `content` - The content of the note.
    /// * `conversation_id` - Optional ID of the associated conversation.
    /// * `message_id` - Optional ID of the associated message.
    /// * `tags` - A slice of tag names to associate with the note.
    ///
    /// # Returns
    /// The ID of the newly created note.
    ///
    /// # Errors
    /// Returns a `StoreError` if any database operation fails.
    pub fn add_note(
        &mut self,
        title: &str,
        content: &str,
        conversation_id: Option<i64>,
        message_id: Option<i64>,
        tags: Vec<&str>,
        metadata: Option<serde_json::Value>,
    ) -> Result<i64, StoreError> {
        let content_hash = format!("{:x}", xxh32(content.as_bytes(), 0));

        // 检查是否存在重复笔记
        let exists = self
            .conn
            .query_row(
                "SELECT id FROM notes
            WHERE content_hash = ?1
            AND (conversation_id = ?2 OR (?2 IS NULL AND conversation_id IS NULL))
            AND (message_id = ?3 OR (?3 IS NULL AND message_id IS NULL))
            AND deleted_at IS NULL
            LIMIT 1",
                params![&content_hash, conversation_id, message_id],
                |_| Ok(true),
            )
            .optional()?;

        if exists.unwrap_or(false) {
            return Err(StoreError::AlreadyExists(
                t!("chat.note_already_exists").into(),
            ));
        }

        let metadata_str = metadata
            .map(|m| serde_json::to_string(&m))
            .transpose()
            .map_err(|e| {
                StoreError::JsonError(
                    t!("db.json_serialize_failed_metadata", error = e.to_string()).to_string(),
                )
            })?;
        let now = Utc::now().timestamp();
        let tx = self.conn.transaction()?;

        // 插入笔记
        tx.execute(
            "INSERT INTO notes (tags,title, content, content_hash, conversation_id, message_id, created_at, updated_at, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6,?7, ?8)",
            params![tags.join(","), title, content, content_hash, conversation_id, message_id, now, metadata_str],
        )?;

        let note_id = tx.last_insert_rowid();

        // 处理标签
        for tag_name in tags {
            // 插入或获取标签
            tx.execute(
                "INSERT INTO note_tag_items (name,note_count, created_at)
                 VALUES (?1, 1, ?2)
                 ON CONFLICT(name) DO UPDATE SET note_count = note_count + 1",
                params![tag_name, now],
            )?;

            let tag_id = {
                let id: i64 = tx.query_row(
                    "SELECT id FROM note_tag_items WHERE name = ?1",
                    params![tag_name],
                    |row| row.get(0),
                )?;
                id
            };

            // 建立笔记和标签的关联
            tx.execute(
                "INSERT INTO note_tag_relations (note_id, tag_id, created_at)
                 VALUES (?1, ?2, ?3)",
                params![note_id, tag_id, now],
            )?;
        }

        tx.commit()?;
        Ok(note_id)
    }

    /// Deletes a note and updates the associated tag counts.
    ///
    /// # Arguments
    ///
    /// * `note_id` - The ID of the note to delete.
    ///
    /// # Errors
    /// Returns a `StoreError` if any database operation fails.
    pub fn delete_note(&mut self, note_id: i64) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;

        // Handle stmt in a separate scope to ensure it gets dropped before the transaction is committed
        let tag_ids = {
            let mut stmt =
                tx.prepare("SELECT tag_id FROM note_tag_relations WHERE note_id = ?1")?;
            let ids: Vec<i64> = stmt
                .query_map(params![note_id], |row| row.get(0))?
                .collect::<Result<Vec<i64>, _>>()?;
            ids
        };

        // update note tag count
        for tag_id in tag_ids {
            tx.execute(
                "UPDATE note_tag_items SET note_count = note_count - 1 WHERE id = ?1 and note_count > 0",
                params![tag_id],
            )?;
        }

        // remove tag relations first
        tx.execute(
            "DELETE FROM note_tag_relations WHERE note_id = ?1",
            params![note_id],
        )?;

        // delete unused tags
        tx.execute("DELETE from note_tag_items where note_count=0", [])?;

        // remove note
        tx.execute("DELETE FROM notes WHERE id = ?1", params![note_id])?;

        tx.commit()?;
        Ok(())
    }

    /// Gets a note by its ID.
    ///
    /// # Arguments
    ///
    /// * `note_id` - The ID of the note to retrieve.
    ///
    /// # Returns
    /// The `Note` instance if found, or `None` if not found.
    ///
    /// # Errors
    /// Returns a `StoreError` if any database operation fails.
    pub fn get_note(&self, note_id: i64) -> Result<Note, StoreError> {
        let mut stmt = self.conn.prepare("SELECT * FROM notes WHERE id = ?1")?;
        let note = stmt.query_row(params![note_id], |row| {
            let metadata_str: Option<String> = row.get("metadata")?;
            let metadata = metadata_str.and_then(|s| serde_json::from_str(&s).ok());
            Ok(Note {
                id: row.get("id")?,
                tags: row
                    .get::<_, Option<String>>("tags")?
                    .unwrap_or_default()
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
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
        })?;

        Ok(note)
    }

    /// Retrieves a list of notes, optionally filtered by tag.
    ///
    /// # Arguments
    ///
    /// * `tag_id` - Optional tag ID to filter notes by.
    ///
    /// # Returns
    /// A vector of `Note` instances matching the filter criteria.
    ///
    /// # Errors
    /// Returns a `StoreError` if any database operation fails.
    pub fn get_notes(&self, tag_id: Option<i64>) -> Result<Vec<Note>, StoreError> {
        let sql = match tag_id {
            Some(_) => {
                "
                SELECT n.* FROM notes n
                INNER JOIN note_tag_relations r ON n.id = r.note_id
                WHERE r.tag_id = ?1 AND n.deleted_at IS NULL
                ORDER BY n.created_at DESC"
            }
            None => {
                "
                SELECT * FROM notes
                WHERE deleted_at IS NULL
                ORDER BY created_at DESC"
            }
        };

        let mut stmt = self.conn.prepare(sql)?;

        // 提取闭包到一个变量
        let map_fn = |row: &rusqlite::Row| -> SqliteResult<Note> {
            let metadata_str: Option<String> = row.get("metadata")?;
            let metadata = metadata_str.and_then(|s| serde_json::from_str(&s).ok());

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
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect(),
                metadata,
            })
        };

        let notes = match tag_id {
            Some(tid) => stmt.query_map(params![tid], map_fn)?,
            None => stmt.query_map([], map_fn)?,
        };

        notes
            .collect::<SqliteResult<Vec<Note>>>()
            .map_err(StoreError::from)
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
    pub fn search_notes(&self, kw: &str) -> Result<Vec<Note>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT n.* FROM notes n WHERE n.title LIKE ?1 ORDER BY n.updated_at DESC")?;

        let search_pattern = format!("%{}%", kw);
        let notes = stmt
            .query_map(params![search_pattern], |row| {
                let metadata_str: Option<String> = row.get("metadata")?;
                let metadata = metadata_str.and_then(|s| serde_json::from_str(&s).ok());

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
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                        .collect(),
                    metadata,
                })
            })?
            .collect::<Result<Vec<Note>, _>>()?;

        Ok(notes)
    }

    /// Retrieves a list of all tags that have associated notes.
    ///
    /// # Returns
    /// A vector of `NoteTag` instances.
    ///
    /// # Errors
    /// Returns a `StoreError` if any database operation fails.
    pub fn get_tags(&self) -> Result<Vec<NoteTag>, StoreError> {
        let mut stmt = self.conn.prepare(
            "
            SELECT * FROM note_tag_items
            ORDER BY name ASC",
        )?;

        let tags = stmt.query_map([], |row| {
            Ok(NoteTag {
                id: row.get("id")?,
                name: row.get("name")?,
                note_count: row.get("note_count")?,
                created_at: row.get("created_at")?,
            })
        })?;

        tags.collect::<SqliteResult<Vec<NoteTag>>>()
            .map_err(StoreError::from)
    }
}
