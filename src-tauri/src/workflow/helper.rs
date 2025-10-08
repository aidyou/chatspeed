use std::sync::Arc;
use tauri::{AppHandle, Emitter as _, Listener as _, Manager as _};
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration, Instant};

use crate::ai::chat::openai::OpenAIChat;
use crate::ai::interaction::chat_completion::PendingWorkflow;
use crate::ai::traits::chat::{AiChatTrait as _, MCPToolDeclaration, MessageType};
use crate::ai::{interaction::chat_completion::ChatState, traits::chat::ChatResponse};
use crate::db::MainStore;

const BATCH_SIZE: usize = 100;
const THROTTLE_DURATION: Duration = Duration::from_millis(100);

/// Listens for the frontend to signal it's ready to receive a workflow stream.
pub fn listen_for_workflow_ready(app_handle: &AppHandle) {
    let handle = app_handle.clone();
    app_handle.listen("frontend_ready_for_stream", move |event| {
        if !event.payload().is_empty() {
            if let Ok(payload_val) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                if let Some(stream_id) = payload_val.get("stream_id").and_then(|v| v.as_str()) {
                    let chat_state: tauri::State<Arc<ChatState>> = handle.state();
                    if let Some((_, pending_workflow)) = chat_state
                        .pending_workflow_chat_completions
                        .remove(stream_id)
                    {
                        // Frontend is ready, now we can spawn the actual chat task
                        run_chat_task(pending_workflow, stream_id.to_string());
                    }
                }
            } else {
                log::error!(
                    "Failed to parse frontend_ready_for_stream payload: {}",
                    event.payload()
                );
            }
        }
    });
}

/// Spawns the actual async task that performs the chat completion with batching.
fn run_chat_task(pending_workflow: PendingWorkflow, stream_id: String) {
    tokio::spawn(async move {
        // Unpack context
        let app_handle = pending_workflow.app_handle;
        let main_store = app_handle
            .state::<Arc<std::sync::RwLock<MainStore>>>()
            .inner()
            .clone();
        let chat_state = app_handle.state::<Arc<ChatState>>().inner();
        let payload = pending_workflow.payload;
        let event_name = format!("workflow_stream://{}", &stream_id);

        // Batching Logic Setup
        let (tx, mut rx) = mpsc::channel::<Arc<ChatResponse>>(1000);

        // Task for batching and emitting
        let batch_app_handle = app_handle.clone();
        let batch_event_name = event_name.clone();
        tokio::spawn(async move {
            let mut batch = Vec::<ChatResponse>::with_capacity(BATCH_SIZE);
            let mut last_emit = Instant::now();

            while let Some(chunk) = rx.recv().await {
                // Non-text messages are sent immediately
                if chunk.r#type != MessageType::Text && chunk.r#type != MessageType::Reasoning {
                    // Send current batch if not empty
                    if !batch.is_empty() {
                        if let Err(e) =
                            emit_batch(&batch_app_handle, &batch_event_name, &batch).await
                        {
                            log::error!("Failed to emit batch: {}", e);
                        }
                        batch.clear();
                    }
                    // Send the special message
                    if let Err(e) = emit_chunk(&batch_app_handle, &batch_event_name, chunk).await {
                        log::error!("Failed to emit AI chunk: {}", e);
                    }
                    continue;
                }

                let should_emit =
                    batch.len() >= BATCH_SIZE || last_emit.elapsed() >= THROTTLE_DURATION;

                if !batch.is_empty() && (batch[0].r#type != chunk.r#type || should_emit) {
                    if let Err(e) = emit_batch(&batch_app_handle, &batch_event_name, &batch).await {
                        log::error!("Failed to emit batch: {}", e);
                    }
                    batch.clear();
                    last_emit = Instant::now();
                }

                // Add new message to batch
                batch.push(chunk.as_ref().clone());

                // Add a small delay to prevent CPU overload
                sleep(Duration::from_millis(1)).await;
            }

            // Final flush after the loop ends
            if !batch.is_empty() {
                if let Err(e) = emit_batch(&batch_app_handle, &batch_event_name, &batch).await {
                    log::error!("Failed to emit final batch: {}", e);
                }
            }
        });

        // Tool and Chat Setup
        let tool_manager = &chat_state.tool_manager;
        let all_rust_tools = tool_manager
            .get_tool_calling_spec(None)
            .await
            .unwrap_or_default();
        let available_rust_tools: Vec<MCPToolDeclaration> = all_rust_tools
            .into_iter()
            .filter(|tool| payload.available_tools.contains(&tool.name))
            .collect();

        // ts tools
        let mut combined_tools = payload.ts_tools;
        combined_tools.extend(available_rust_tools);

        // Callback and Chat Execution
        let chat = OpenAIChat::new(main_store);
        let callback = move |chunk: Arc<ChatResponse>| {
            if let Err(e) = tx.try_send(chunk) {
                log::error!("Failed to send workflow chunk to batching channel: {}", e);
            }
        };

        let _ = chat
            .chat(
                payload.provider_id,
                &payload.model_id,
                stream_id,
                payload.messages,
                Some(combined_tools),
                None,
                callback,
            )
            .await;
    });
}

/// Helper functions for batching
async fn emit_chunk(
    window: &AppHandle,
    event_name: &str,
    chunk: Arc<ChatResponse>,
) -> Result<(), tauri::Error> {
    window.emit(event_name, serde_json::json!(&chunk))
}

async fn emit_batch(
    window: &AppHandle,
    event_name: &str,
    batch: &[ChatResponse],
) -> Result<(), tauri::Error> {
    if batch.is_empty() {
        return Ok(());
    }
    let mut combined = String::with_capacity(batch.iter().map(|chunk| chunk.chunk.len()).sum());
    for chunk in batch {
        combined.push_str(&chunk.chunk);
    }
    let first_msg = &batch[0];
    let combined_chunk = Arc::new(ChatResponse {
        chat_id: first_msg.chat_id.clone(),
        chunk: combined,
        metadata: first_msg.metadata.clone(),
        r#type: first_msg.r#type.clone(),
        finish_reason: first_msg.finish_reason.clone(),
    });
    emit_chunk(window, event_name, combined_chunk).await
}
