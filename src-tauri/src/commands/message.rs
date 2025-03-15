//!
//! This module contains Tauri commands for managing chat conversations and messages
//! within the chat store. It provides functionalities to get, add, update, and delete
//! conversations and messages. The commands are designed to be invoked from the
//! frontend, allowing seamless interaction with the chat capabilities of the
//! application.
//!
//! ## Overview
//!
//! - **Conversations**: Functions to manage conversations, including adding, updating,
//!   deleting, and retrieving conversations.
//! - **Messages**: Functions to manage messages, including adding, updating,
//!   deleting, and retrieving messages.
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
//! // Call from frontend to get all conversations:
//! const conversations = await invoke('get_all_conversations');
//! console.log(conversations);
//! ```

use crate::db::{Conversation, MainStore};

use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tauri::{command, Emitter, Manager, State};

/// Get all conversations
///
/// Retrieves a list of all conversations from the chat store.
///
/// # Arguments
/// - `state` - The state of the chat store, automatically injected by Tauri
///
/// # Returns
/// * `Result<Vec<Conversation>, String>` - A vector of conversations or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// const conversations = await invoke('get_all_conversations');
/// console.log(conversations);
/// ```
#[command]
pub fn get_all_conversations(
    state: State<Arc<Mutex<MainStore>>>,
) -> Result<Vec<Conversation>, String> {
    let main_store = state.lock().map_err(|e| e.to_string())?;
    main_store
        .get_all_conversations()
        .map_err(|e| e.to_string())
}

/// Get a conversation by ID
///
/// Retrieves a conversation by its ID from the chat store.
///
/// # Arguments
/// - `state` - The state of the chat store, automatically injected by Tauri
/// - `id` - The ID of the conversation
///
/// # Returns
/// * `Result<Conversation, String>` - A conversation or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// const conversation = await invoke('get_conversation_by_id', { id: 1 });
/// console.log(conversation);
/// ```
#[command]
pub fn get_conversation_by_id(
    state: State<Arc<Mutex<MainStore>>>,
    id: i64,
) -> Result<Conversation, String> {
    let main_store = state.lock().map_err(|e| e.to_string())?;
    main_store
        .get_conversation_by_id(id)
        .map_err(|e| e.to_string())
}

/// Add a new conversation
///
/// Adds a new conversation to the chat store.
///
/// # Arguments
/// - `state` - The state of the chat store, automatically injected by Tauri
/// - `title` - The title of the conversation to add
///
/// # Returns
/// * `Result<i64, String>` - The ID of the added conversation or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// const newConversationId = await invoke('add_conversation', { title: 'New Conversation' });
/// console.log(`Added Conversation with ID: ${newConversationId}`);
/// ```
#[command]
pub fn add_conversation(state: State<Arc<Mutex<MainStore>>>, title: String) -> Result<i64, String> {
    let main_store = state.lock().map_err(|e| e.to_string())?;
    main_store
        .add_conversation(title)
        .map_err(|e| e.to_string())
}

/// Update conversation favorite status
///
/// Updates the favorite status of a conversation in the chat store.
///
/// # Arguments
/// - `state` - The state of the chat store, automatically injected by Tauri
/// - `id` - The ID of the conversation to update
/// - `is_favorite` - The new favorite status
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
/// await invoke('update_conversation_favorite', { id: 1, isFavorite: true });
/// console.log('Conversation favorite status updated successfully');
/// ```
#[command]
pub fn update_conversation(
    state: State<Arc<Mutex<MainStore>>>,
    id: i64,
    title: Option<String>,
    is_favorite: Option<bool>,
) -> Result<(), String> {
    let main_store = state.lock().map_err(|e| e.to_string())?;
    main_store
        .update_conversation(id, title, is_favorite)
        .map_err(|e| e.to_string())
}

/// Delete a conversation
///
/// Removes a conversation from the chat store by its ID.
///
/// # Arguments
/// - `state` - The state of the chat store, automatically injected by Tauri
/// - `id` - The ID of the conversation to delete
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
/// await invoke('delete_conversation', { id: 1 });
/// console.log('Conversation deleted successfully');
/// ```
#[command]
pub fn delete_conversation(state: State<Arc<Mutex<MainStore>>>, id: i64) -> Result<(), String> {
    let main_store = state.lock().map_err(|e| e.to_string())?;
    main_store
        .delete_conversation(id)
        .map_err(|e| e.to_string())
}

/// Get messages for a conversation
///
/// Retrieves all messages for a specific conversation from the chat store.
///
/// # Arguments
/// - `state` - The state of the chat store, automatically injected by Tauri
/// - `conversation_id` - The ID of the conversation
///
/// # Returns
/// * `Result<Vec<Message>, String>` - A vector of messages or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// const messages = await invoke('get_messages_for_conversation', { conversationId: 1 });
/// console.log(messages);
/// ```
#[command]
pub fn get_messages_for_conversation(
    window: tauri::Window,
    state: State<Arc<Mutex<MainStore>>>,
    conversation_id: i64,
    label: Option<String>,
) -> Result<(), String> {
    let label = label.unwrap_or(window.label().to_string());
    let main_store = state.lock().map_err(|e| e.to_string())?;
    let messages = main_store
        .get_messages_for_conversation(conversation_id)
        .map_err(|e| e.to_string())?;

    let app = window.app_handle();
    for m in messages.iter() {
        send_message(app.clone(), &label, json!(m.clone()), false);
    }
    send_message(app.clone(), &label, json!({}), true);
    Ok(())
}

/// Add a new message
///
/// Adds a new message to a conversation in the chat store.
///
/// # Arguments
/// - `state` - The state of the chat store, automatically injected by Tauri
/// - `conversation_id` - The ID of the conversation
/// - `role` - The role of the message sender
/// - `content` - The content of the message
///
/// # Returns
/// * `Result<i64, String>` - The ID of the added message or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// const newMessageId = await invoke('add_message', {
///     conversationId: 1,
///     role: 'user',
///     content: 'Hello, AI!'
/// });
/// console.log(`Added Message with ID: ${newMessageId}`);
/// ```
#[command]
pub fn add_message(
    state: State<Arc<Mutex<MainStore>>>,
    conversation_id: i64,
    role: String,
    content: String,
    metadata: Option<serde_json::Value>,
) -> Result<i64, String> {
    let main_store = state.lock().map_err(|e| e.to_string())?;
    main_store
        .add_message(conversation_id, role, content, metadata)
        .map_err(|e| e.to_string())
}

/// Delete a message
///
/// Removes a message from the chat store by its ID.
///
/// # Arguments
/// - `state` - The state of the chat store, automatically injected by Tauri
/// - `id` - The ID of the message to delete
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
/// await invoke('delete_message', { id: 1 });
/// console.log('Message deleted successfully');
/// ```
#[command]
pub fn delete_message(state: State<Arc<Mutex<MainStore>>>, id: i64) -> Result<(), String> {
    let main_store = state.lock().map_err(|e| e.to_string())?;
    main_store.delete_message(id).map_err(|e| e.to_string())
}
/// Update the metadata of a message
///
/// Updates the metadata of a message in the chat store.
///
/// # Arguments
/// - `state` - The state of the chat store, automatically injected by Tauri
/// - `id` - The ID of the message to update
/// - `metadata` - The new metadata to set for the message
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
/// await invoke('update_message_metadata', { id: 1, metadata: { contextClear: true } });
/// console.log('Message metadata updated successfully');
#[command]
pub fn update_message_metadata(
    state: State<Arc<Mutex<MainStore>>>,
    id: i64,
    metadata: serde_json::Value,
) -> Result<(), String> {
    let main_store = state.lock().map_err(|e| e.to_string())?;
    main_store
        .update_message_metadata(id, Some(metadata))
        .map_err(|e| e.to_string())
}

/// Sends a conversation message to the frontend with the specified label and message content.
///
/// # Arguments
/// - `app` - The Tauri app handle
/// - `label` - The label of the conversation
/// - `message` - The message content
#[tauri::command]
pub fn send_message(app: tauri::AppHandle, label: &str, message: Value, done: bool) {
    let mut payload: HashMap<String, Value> = HashMap::new();
    payload.insert("label".to_string(), Value::String(label.to_string()));
    payload.insert("message".to_string(), message);
    payload.insert("done".to_string(), Value::Bool(done));

    let _ = app.emit("chat_message", payload);
}
