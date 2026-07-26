//!
//! This module contains Tauri commands for managing notes and tags within the note store.
//! It provides functionalities to create, retrieve, update, and delete notes and their
//! associated tags. The commands are designed to be invoked from the frontend,
//! allowing seamless interaction with the note-taking capabilities of the application.
//!
//! ## Overview
//!
//! - **Notes**: Functions to manage notes, including adding, retrieving, and deleting notes.
//! - **Tags**: Functions to manage note tags and their associations with notes.
//!
//! ## Usage
//!
//! The commands can be invoked from the frontend using Tauri's `invoke`
//! function. Each command is annotated with detailed documentation, including
//! parameters, return types, and examples of usage.
//!
//! ## Example
//!
//! ```js
//! import { invoke } from '@tauri-apps/api/core'
//! // Call from frontend to get all notes with a specific tag:
//! const notes = await invoke('get_notes', { tagId: 1 });
//! console.log(notes);
//! ```

use crate::db::{MainStore, Note, NoteTag};
use std::sync::Arc;
use tauri::{command, State};

use crate::error::{AppError, Result};

/// Add a new note
///
/// Creates a new note with the specified title, content, and tags.
///
/// # Arguments
/// - `state` - The state of the note store, automatically injected by Tauri
/// - `title` - The title of the note
/// - `content` - The content of the note
/// - `conversation_id` - Optional ID of the associated conversation
/// - `message_id` - Optional ID of the associated message
/// - `tags` - Comma-separated list of tags
///
/// # Returns
/// * `Result<(), String>` - Ok if successful or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// await invoke('add_note', {
///     title: 'My Note',
///     content: 'Note content',
///     conversationId: 1,
///     messageId: 2,
///     tags: 'rust,programming'
/// });
/// ```
#[command]
pub async fn add_note(
    state: State<'_, Arc<MainStore>>,
    title: String,
    content: String,
    conversation_id: Option<i64>,
    message_id: Option<i64>,
    tags: Vec<String>,
    metadata: Option<serde_json::Value>,
) -> Result<()> {
    let runtime = {
        let main_store = state.read()?;
        main_store.db_runtime().map_err(AppError::Db)?
    };
    MainStore::add_note_with_runtime(
        runtime,
        title,
        content,
        conversation_id,
        message_id,
        tags,
        metadata,
    )
    .await
    .map_err(AppError::Db)?;
    Ok(())
}

/// Get all tags
///
/// Retrieves a list of all tags from the note store.
///
/// # Arguments
/// - `state` - The state of the note store, automatically injected by Tauri
///
/// # Returns
/// * `Result<Vec<NoteTag>, String>` - A vector of tags or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// const tags = await invoke('get_tags');
/// console.log(tags);
/// ```
#[command]
pub async fn get_tags(state: State<'_, Arc<MainStore>>) -> Result<Vec<NoteTag>> {
    let runtime = {
        let main_store = state.read()?;
        main_store.db_runtime().map_err(AppError::Db)?
    };
    MainStore::get_tags_with_runtime(runtime)
        .await
        .map_err(AppError::Db)
}

/// Get notes by tag ID
///
/// Retrieves all notes associated with a specific tag.
///
/// # Arguments
/// - `state` - The state of the note store, automatically injected by Tauri
/// - `tag_id` - The ID of the tag to filter notes by
///
/// # Returns
/// * `Result<Vec<Note>, String>` - A vector of notes or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// const notes = await invoke('get_notes', { tagId: 1 });
/// console.log(notes);
/// ```
#[command]
pub async fn get_notes(state: State<'_, Arc<MainStore>>, tag_id: Option<i64>) -> Result<Vec<Note>> {
    let runtime = {
        let main_store = state.read()?;
        main_store.db_runtime().map_err(AppError::Db)?
    };
    MainStore::get_notes_with_runtime(runtime, tag_id)
        .await
        .map_err(AppError::Db)
}

/// Get a note by ID
///
/// Retrieves a specific note by its ID.
///
/// # Arguments
/// - `state` - The state of the note store, automatically injected by Tauri
/// - `id` - The ID of the note to retrieve
///
/// # Returns
/// * `Result<Note, String>` - The requested note or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// const note = await invoke('get_note', { id: 1 });
/// console.log(note);
/// ```
#[command]
pub async fn get_note(state: State<'_, Arc<MainStore>>, id: i64) -> Result<Note> {
    let runtime = {
        let main_store = state.read()?;
        main_store.db_runtime().map_err(AppError::Db)?
    };
    MainStore::get_note_with_runtime(runtime, id)
        .await
        .map_err(AppError::Db)
}

/// Delete a note
///
/// Removes a note and its tag associations from the store.
///
/// # Arguments
/// - `state` - The state of the note store, automatically injected by Tauri
/// - `id` - The ID of the note to delete
///
/// # Returns
/// * `Result<(), String>` - Ok if successful or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// await invoke('delete_note', { id: 1 });
/// console.log('Note deleted successfully');
/// ```
#[command]
pub async fn delete_note(state: State<'_, Arc<MainStore>>, id: i64) -> Result<()> {
    let runtime = {
        let main_store = state.read()?;
        main_store.db_runtime().map_err(AppError::Db)?
    };
    MainStore::delete_note_with_runtime(runtime, id)
        .await
        .map_err(AppError::Db)
}

/// Search notes
///
/// Searches for notes by keyword in their titles.
///
/// # Arguments
/// - `state` - The state of the note store, automatically injected by Tauri
/// - `kw` - The keyword to search for in note titles
///
/// # Returns
/// * `Result<Vec<Note>, String>` - Matching notes or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// const notes = await invoke('search_notes', { kw: 'rust' });
/// console.log('Found matching notes:', notes);
/// ```
#[command]
pub async fn search_notes(state: State<'_, Arc<MainStore>>, kw: String) -> Result<Vec<Note>> {
    let runtime = {
        let main_store = state.read()?;
        main_store.db_runtime().map_err(AppError::Db)?
    };
    MainStore::search_notes_with_runtime(runtime, kw)
        .await
        .map_err(AppError::Db)
}
