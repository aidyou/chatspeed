//! Note and tag Tauri commands.
//!
//! Notes and tags are owned by the runtime store. These wrappers only translate
//! the Tauri wire into `/control/v1/data-commands/*` calls through the
//! [`RuntimeSupervisor`]; there is no local store fallback, so an unreachable
//! runtime fails instead of writing to a second owner.

use std::sync::Arc;

use tauri::{command, State};

use crate::db::{Note, NoteTag};
use crate::runtime_client::RuntimeSupervisor;
use crate::runtime_data::AddNoteBody;

/// Adds a new note.
#[command]
pub async fn add_note(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    title: String,
    content: String,
    conversation_id: Option<i64>,
    message_id: Option<i64>,
    tags: Vec<String>,
    metadata: Option<serde_json::Value>,
) -> Result<(), String> {
    crate::runtime_data::add_note(
        supervisor.inner().as_ref(),
        AddNoteBody {
            title,
            content,
            conversation_id,
            message_id,
            tags,
            metadata,
        },
    )
    .await
}

/// Returns all tags.
#[command]
pub async fn get_tags(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Vec<NoteTag>, String> {
    crate::runtime_data::get_tags(supervisor.inner().as_ref()).await
}

/// Returns notes, optionally filtered by tag id.
#[command]
pub async fn get_notes(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    tag_id: Option<i64>,
) -> Result<Vec<Note>, String> {
    crate::runtime_data::get_notes(supervisor.inner().as_ref(), tag_id).await
}

/// Returns one note by id.
#[command]
pub async fn get_note(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
) -> Result<Note, String> {
    crate::runtime_data::get_note(supervisor.inner().as_ref(), id).await
}

/// Deletes a note.
#[command]
pub async fn delete_note(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
) -> Result<(), String> {
    crate::runtime_data::delete_note(supervisor.inner().as_ref(), id).await
}

/// Searches notes by keyword.
#[command]
pub async fn search_notes(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    kw: String,
) -> Result<Vec<Note>, String> {
    crate::runtime_data::search_notes(supervisor.inner().as_ref(), kw).await
}
