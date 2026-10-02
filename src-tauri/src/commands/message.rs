//! Conversation and message Tauri commands.
//!
//! Conversations and messages are owned by the runtime store, so the CRUD and
//! the sensitive-filtered message insert go through
//! `/control/v1/data-commands/*`. Window delivery (`chat_message` events) stays
//! local: the desktop reads the messages from the runtime and emits them itself.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use tauri::{command, Emitter, Manager, State};

use crate::db::Conversation;
use crate::runtime_client::RuntimeSupervisor;
use crate::runtime_data::{AddMessageBody, UpdateConversationBody, UpdateMessageMetadataBody};

/// Returns all conversations.
#[command]
pub async fn get_all_conversations(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Vec<Conversation>, String> {
    crate::runtime_data::get_all_conversations(supervisor.inner().as_ref()).await
}

/// Returns one conversation by id.
#[command]
pub async fn get_conversation_by_id(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
) -> Result<Conversation, String> {
    crate::runtime_data::get_conversation_by_id(supervisor.inner().as_ref(), id).await
}

/// Adds a conversation and returns its id.
#[command]
pub async fn add_conversation(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    title: String,
) -> Result<i64, String> {
    crate::runtime_data::add_conversation(supervisor.inner().as_ref(), title).await
}

/// Updates a conversation's title and/or favorite flag.
#[command]
pub async fn update_conversation(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
    title: Option<String>,
    is_favorite: Option<bool>,
) -> Result<(), String> {
    crate::runtime_data::update_conversation(
        supervisor.inner().as_ref(),
        UpdateConversationBody {
            id,
            title,
            is_favorite,
        },
    )
    .await
}

/// Deletes a conversation.
#[command]
pub async fn delete_conversation(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
) -> Result<(), String> {
    crate::runtime_data::delete_conversation(supervisor.inner().as_ref(), id).await
}

/// Streams a conversation's messages to the frontend on the `chat_message`
/// event, following the historical per-message plus terminal convention.
#[command]
pub async fn get_messages_for_conversation(
    window: tauri::Window,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    conversation_id: i64,
    window_label: Option<String>,
) -> Result<(), String> {
    let label = window_label.unwrap_or_else(|| window.label().to_string());
    let messages = crate::runtime_data::get_messages_for_conversation(
        supervisor.inner().as_ref(),
        conversation_id,
    )
    .await?;

    let app = window.app_handle();
    for message in messages.iter() {
        send_message(app.clone(), &label, message.clone(), false);
    }
    send_message(app.clone(), &label, serde_json::json!({}), true);
    Ok(())
}

/// Adds a message; the runtime performs the sensitive filter and returns
/// `(id, final_content)`.
#[command]
pub async fn add_message(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    conversation_id: i64,
    role: String,
    content: String,
    metadata: Option<serde_json::Value>,
) -> Result<(i64, String), String> {
    crate::runtime_data::add_message(
        supervisor.inner().as_ref(),
        AddMessageBody {
            conversation_id,
            role,
            content,
            metadata,
        },
    )
    .await
}

/// Deletes one or more messages.
#[command]
pub async fn delete_message(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: Vec<i64>,
) -> Result<(), String> {
    crate::runtime_data::delete_message(supervisor.inner().as_ref(), id).await
}

/// Replaces a message's metadata.
#[command]
pub async fn update_message_metadata(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
    metadata: serde_json::Value,
) -> Result<(), String> {
    crate::runtime_data::update_message_metadata(
        supervisor.inner().as_ref(),
        UpdateMessageMetadataBody { id, metadata },
    )
    .await
}

/// Emits one message chunk (or the terminal marker) to the frontend.
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
