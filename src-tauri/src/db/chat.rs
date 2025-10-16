use super::types::{Conversation, Message};
use crate::db::error::StoreError;
use crate::db::main_store::MainStore;

use rusqlite::params;
use rust_i18n::t;
use serde_json::Value;

impl MainStore {
    /// Retrieves a conversation by its ID.
    ///
    /// # Arguments
    ///
    /// * `id` - The ID of the conversation.
    ///
    /// # Returns
    /// A `Conversation` instance.
    ///
    /// # Errors
    /// Returns a `StoreError` if the database operation fails.
    pub fn get_conversation_by_id(&self, id: i64) -> Result<Conversation, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let conversation = conn
            .query_row(
                "SELECT id, title, created_at, is_favorite FROM conversations WHERE id = ?",
                [id],
                |row| {
                    Ok(Conversation {
                        id: row.get("id")?,
                        title: row.get("title")?,
                        created_at: row.get("created_at")?,
                        is_favorite: row.get("is_favorite")?,
                    })
                },
            )
            .map_err(|e| {
                if e == rusqlite::Error::QueryReturnedNoRows {
                    StoreError::NotFound(t!("db.conversation_not_found").to_string())
                } else {
                    StoreError::from(e)
                }
            })?;
        Ok(conversation)
    }

    // TODO: add pagination to get_all_conversations
    /// Retrieves all conversation topics from the database.
    ///
    /// # Returns
    ///
    /// A vector of `Conversation` instances.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database operation fails.
    pub fn get_all_conversations(&self) -> Result<Vec<Conversation>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT id, title, created_at, is_favorite FROM conversations order by id desc",
        )?;
        let conversations = stmt.query_map([], |row| {
            Ok(Conversation {
                id: row.get("id")?,
                title: row.get("title")?,
                created_at: row.get("created_at")?,
                is_favorite: row.get("is_favorite")?,
            })
        })?;
        conversations
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Retrieves all messages for a specific conversation.
    ///
    /// # Arguments
    ///
    /// * `conversation_id` - The ID of the conversation.
    ///
    /// # Returns
    ///
    /// A vector of `Message` instances.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database operation fails.
    pub fn get_messages_for_conversation(
        &self,
        conversation_id: i64,
    ) -> Result<Vec<Message>, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT id, conversation_id, role, content, timestamp, metadata
             FROM messages WHERE conversation_id = ? order by id asc",
        )?;

        let messages = stmt.query_map([conversation_id], |row| {
            let metadata_str: Option<String> = row.get("metadata")?;
            let metadata = metadata_str.and_then(|s| {
                serde_json::from_str(&s)
                    .map_err(|e| {
                        log::warn!(
                            "Failed to parse metadata JSON for message: {}, error: {}",
                            s,
                            e
                        );
                        e
                    })
                    .ok()
            });

            Ok(Message {
                id: row.get("id")?,
                conversation_id: row.get("conversation_id")?,
                role: row.get("role")?,
                content: row.get("content")?,
                timestamp: row.get("timestamp")?,
                metadata,
            })
        })?;

        messages
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Adds a new conversation to the database.
    ///
    /// Inserts a new record into the `conversations` table and returns the generated ID.
    ///
    /// # Arguments
    ///
    /// * `title` - The title of the conversation.
    ///
    /// # Returns
    ///
    /// The ID of the newly inserted conversation.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database operation fails.
    pub fn add_conversation(&self, title: String) -> Result<i64, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute(
            "INSERT INTO conversations (title,is_favorite, created_at) VALUES (?, 0, CURRENT_TIMESTAMP)",
            [title],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Updates the favorite status of a conversation.
    ///
    /// # Arguments
    ///
    /// * `id` - The ID of the conversation to update.
    /// * `title` - The new title of the conversation.
    /// * `is_favorite` - The new favorite status.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database operation fails.
    pub fn update_conversation(
        &self,
        id: i64,
        title: Option<String>,
        is_favorite: Option<bool>,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        if let Some(title) = title {
            conn.execute(
                "UPDATE conversations SET title = ? WHERE id = ?",
                params![title, id],
            )?;
        }
        if let Some(is_favorite) = is_favorite {
            conn.execute(
                "UPDATE conversations SET is_favorite = ? WHERE id = ?",
                params![if is_favorite { 1 } else { 0 }, id],
            )?;
        }
        Ok(())
    }

    /// Deletes a conversation from the database.
    ///
    /// Removes the record with the specified ID from the `conversations` table.
    ///
    /// # Arguments
    ///
    /// * `id` - The ID of the conversation to be deleted.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database operation fails.
    pub fn delete_conversation(&self, id: i64) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        conn.execute("DELETE FROM conversations WHERE id = ?", params![id])?;
        Ok(())
    }

    /// Adds a new message to the database.
    ///
    /// Inserts a new record into the `messages` table and returns the generated ID.
    ///
    /// # Arguments
    ///
    /// * `conversation_id` - The ID of the conversation to which the message belongs.
    /// * `role` - The role of the message sender.
    /// * `content` - The content of the message.
    ///
    /// # Returns
    ///
    /// The ID of the newly inserted message.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database operation fails.
    pub fn add_message(
        &self,
        conversation_id: i64,
        role: String,
        content: String,
        metadata: Option<Value>,
    ) -> Result<i64, StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let metadata_str = metadata
            .map(|m| serde_json::to_string(&m))
            .transpose()
            .map_err(|e| {
                StoreError::JsonError(
                    t!("db.json_serialize_failed_metadata", error = e.to_string()).to_string(),
                )
            })?;

        conn.execute(
            "INSERT INTO messages (conversation_id, role, content, metadata, timestamp)
             VALUES (?, ?, ?, ?, CURRENT_TIMESTAMP)",
            rusqlite::params![conversation_id, role, content, metadata_str],
        )?;

        Ok(conn.last_insert_rowid())
    }

    /// Deletes messages from the database.
    ///
    /// Removes the records with the specified IDs from the `messages` table.
    ///
    /// # Arguments
    ///
    /// * `id` - The IDs of the messages to be deleted.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database operation fails.
    pub fn delete_message(&self, id: Vec<i64>) -> Result<(), StoreError> {
        if id.is_empty() {
            return Ok(());
        }

        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;

        // Create placeholders for the IN clause (?, ?, ? ...)
        let placeholders: Vec<String> = id.iter().map(|_| "?".to_string()).collect();
        let placeholder_str = placeholders.join(",");

        let sql = format!("DELETE FROM messages WHERE id IN ({})", placeholder_str);
        conn.execute(&sql, rusqlite::params_from_iter(id))?;
        Ok(())
    }

    /// Updates the metadata of a message.
    ///
    /// # Arguments
    ///
    /// * `id` - The ID of the message to update.
    /// * `metadata` - The new metadata for the message.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database operation fails.
    pub fn update_message_metadata(
        &self,
        id: i64,
        metadata: Option<Value>,
    ) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        let metadata_str = metadata
            .map(|m| serde_json::to_string(&m))
            .transpose()
            .map_err(|e| {
                StoreError::JsonError(
                    t!("db.json_serialize_failed_metadata", error = e.to_string()).to_string(),
                )
            })?;

        conn.execute(
            "UPDATE messages SET metadata = ? WHERE id = ?",
            rusqlite::params![metadata_str, id],
        )?;
        Ok(())
    }
}
