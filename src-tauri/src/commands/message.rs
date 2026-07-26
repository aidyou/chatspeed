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

use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};
use tauri::{command, Emitter, Manager, State};

use crate::constants::CFG_INTERFACE_LANGUAGE;
use crate::db::{Conversation, MainStore};
use crate::error::{AppError, Result};
use crate::libs::lang::lang_to_iso_639_1;
use crate::sensitive::manager::{FilterManager, SensitiveConfig};
use whatlang::detect;

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
pub async fn get_all_conversations(state: State<'_, Arc<MainStore>>) -> Result<Vec<Conversation>> {
    let runtime = {
        let main_store = &*state;
        main_store.db_runtime().map_err(AppError::Db)?
    };
    MainStore::get_all_conversations_with_runtime(runtime)
        .await
        .map_err(AppError::Db)
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
pub async fn get_conversation_by_id(
    state: State<'_, Arc<MainStore>>,
    id: i64,
) -> Result<Conversation> {
    let runtime = {
        let main_store = &*state;
        main_store.db_runtime().map_err(AppError::Db)?
    };
    MainStore::get_conversation_by_id_with_runtime(runtime, id)
        .await
        .map_err(AppError::Db)
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
pub async fn add_conversation(state: State<'_, Arc<MainStore>>, title: String) -> Result<i64> {
    let runtime = {
        let main_store = &*state;
        main_store.db_runtime().map_err(AppError::Db)?
    };
    MainStore::add_conversation_with_runtime(runtime, title)
        .await
        .map_err(AppError::Db)
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
pub async fn update_conversation(
    state: State<'_, Arc<MainStore>>,
    id: i64,
    title: Option<String>,
    is_favorite: Option<bool>,
) -> Result<()> {
    let runtime = {
        let main_store = &*state;
        main_store.db_runtime().map_err(AppError::Db)?
    };
    MainStore::update_conversation_with_runtime(runtime, id, title, is_favorite)
        .await
        .map_err(AppError::Db)
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
pub async fn delete_conversation(state: State<'_, Arc<MainStore>>, id: i64) -> Result<()> {
    let runtime = {
        let main_store = &*state;
        main_store.db_runtime().map_err(AppError::Db)?
    };
    MainStore::delete_conversation_with_runtime(runtime, id)
        .await
        .map_err(AppError::Db)
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
pub async fn get_messages_for_conversation(
    window: tauri::Window,
    state: State<'_, Arc<MainStore>>,
    conversation_id: i64,
    window_label: Option<String>,
) -> Result<()> {
    let label = window_label.unwrap_or(window.label().to_string());
    let runtime = {
        let main_store = &*state;
        main_store.db_runtime().map_err(AppError::Db)?
    };
    let messages = MainStore::get_messages_for_conversation_with_runtime(runtime, conversation_id)
        .await
        .map_err(AppError::Db)?;

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
pub async fn add_message(
    state: State<'_, Arc<MainStore>>,
    filter_manager: State<'_, FilterManager>,
    conversation_id: i64,
    role: String,
    content: String,
    metadata: Option<serde_json::Value>,
) -> Result<(i64, String)> {
    let (runtime, final_content) = {
        let main_store = &*state;
        let mut final_content = content;
        if role == "user" {
            let sensitive_config: SensitiveConfig =
                main_store.get_config("sensitive_config", SensitiveConfig::default());

            if sensitive_config.enabled {
                let interface_lang: String =
                    main_store.get_config(CFG_INTERFACE_LANGUAGE, "en".to_string());

                let lang_info = detect(&final_content);
                let detected_code = if let Some(info) = lang_info {
                    lang_to_iso_639_1(&info.lang().code()).unwrap_or("en")
                } else {
                    "en"
                };
                let languages = vec![detected_code, interface_lang.as_str()];

                final_content =
                    filter_manager.filter_text(&final_content, &languages, &sensitive_config);
            }
        }
        (
            main_store.db_runtime().map_err(AppError::Db)?,
            final_content,
        )
    };

    let id = MainStore::add_message_with_runtime(
        runtime,
        conversation_id,
        role,
        final_content.clone(),
        metadata,
    )
    .await
    .map_err(AppError::Db)?;

    Ok((id, final_content))
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
pub async fn delete_message(state: State<'_, Arc<MainStore>>, id: Vec<i64>) -> Result<()> {
    let runtime = {
        let main_store = &*state;
        main_store.db_runtime().map_err(AppError::Db)?
    };
    MainStore::delete_message_with_runtime(runtime, id)
        .await
        .map_err(AppError::Db)
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
pub async fn update_message_metadata(
    state: State<'_, Arc<MainStore>>,
    id: i64,
    metadata: serde_json::Value,
) -> Result<()> {
    let runtime = {
        let main_store = &*state;
        main_store.db_runtime().map_err(AppError::Db)?
    };
    MainStore::update_message_metadata_with_runtime(runtime, id, Some(metadata))
        .await
        .map_err(AppError::Db)
}

/// Sends a conversation message to the frontend with the specified label and message content.
///
/// # Arguments
/// - `app` - The Tauri app handle
/// - `label` - The label of the conversation
/// - `message` - The message content
#[tauri::command]
pub fn send_message(app: tauri::AppHandle, window_label: &str, message: Value, done: bool) {
    let mut payload: HashMap<String, Value> = HashMap::new();
    payload.insert(
        "windowLabel".to_string(),
        Value::String(window_label.to_string()),
    );
    payload.insert("message".to_string(), message);
    payload.insert("done".to_string(), Value::Bool(done));

    let _ = app.emit("chat_message", payload);
}
