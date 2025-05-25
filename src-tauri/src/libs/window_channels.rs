use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration, Instant};

use crate::ai::traits::chat::ChatResponse;
use crate::ai::traits::chat::MessageType;

const BATCH_SIZE: usize = 100; // Maximum number of messages to batch
const THROTTLE_DURATION: Duration = Duration::from_millis(100);

/// Define a structure to hold the channel sender for each window
pub struct WindowChannels {
    // The parameter is a tuple of (chat_id, chunk, is_error, is_done,is_reasoning, metadata)
    channels: Arc<Mutex<HashMap<String, mpsc::Sender<Arc<ChatResponse>>>>>,
}

impl WindowChannels {
    /// Create a new instance of WindowChannels
    pub fn new() -> Self {
        Self {
            channels: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Emit an AI chat stream message to the specified window
    ///
    /// # Arguments
    /// * `window`: The Tauri window to send the message to
    /// * `chunk`: message to send
    async fn emit_chat_stream(
        window: &tauri::Window,
        chunk: Arc<ChatResponse>,
    ) -> Result<(), tauri::Error> {
        window.emit("chat_stream", json!(&chunk)).map_err(|e| {
            log::error!("Failed to serialize AI chunk: {}", e);
            e
        })
    }

    /// Emit a batch of AI chunks as a single message
    ///
    /// # Arguments
    /// * `window`: The Tauri window to send the message to
    /// * `batch`: The batch of AI chunks to send
    async fn emit_batch(
        window: &tauri::Window,
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
        Self::emit_chat_stream(
            window,
            Arc::new(ChatResponse {
                chat_id: first_msg.chat_id.clone(),
                chunk: combined,
                metadata: first_msg.metadata.clone(),
                r#type: first_msg.r#type.clone(),
            }),
        )
        .await
    }

    /// Get or create a channel for the specified window.
    ///
    /// # Arguments
    /// * `window`: The Tauri window for which the channel is being requested.
    ///
    /// # Returns
    /// Returns a sender for the channel associated with the specified window.
    pub async fn get_or_create_channel(
        &self,
        window: tauri::Window,
    ) -> Result<mpsc::Sender<Arc<ChatResponse>>, String> {
        let mut channels = self.channels.lock().await;
        let window_label = window.label().to_string();

        Ok(channels
            .entry(window_label.clone())
            .or_insert_with(|| {
                let (tx, mut rx) = mpsc::channel::<Arc<ChatResponse>>(1000);

                let window = window.clone();
                tokio::spawn(async move {
                    let mut batch = Vec::<ChatResponse>::with_capacity(BATCH_SIZE as usize);
                    let mut last_emit = Instant::now();
                    // let mut is_first: bool = true;

                    while let Some(chunk) = rx.recv().await {
                        // For error or done messages: they should be sent immediately
                        // 1. Send current batch first
                        // 2. Then send the error/done/step/reference message separately
                        //
                        // Note: Reference message is an array, so can't combine simply!!!
                        if chunk.r#type == MessageType::Finished
                            || chunk.r#type == MessageType::Error
                            || chunk.r#type == MessageType::Step
                            || chunk.r#type == MessageType::Reference
                            || chunk.r#type == MessageType::Log
                            || chunk.r#type == MessageType::Plan
                        {
                            #[cfg(debug_assertions)]
                            log::debug!(
                                "sending message: {}, type: {:?}",
                                chunk.chunk,
                                chunk.r#type
                            );

                            // Send current batch if not empty
                            if !batch.is_empty() {
                                if let Err(e) = Self::emit_batch(&window, &batch).await {
                                    log::error!("Failed to emit batch: {}", e);
                                }
                                batch.clear();
                            }

                            // Send the error/done message
                            if let Err(e) = Self::emit_chat_stream(&window, chunk).await {
                                log::error!("Failed to emit AI chunk: {}", e);
                            }
                            continue;
                        }

                        // Check if we should emit based on batch size or time
                        let should_emit =
                            batch.len() >= BATCH_SIZE || last_emit.elapsed() >= THROTTLE_DURATION;

                        // Check if new message is of different type
                        if !batch.is_empty() && (batch[0].r#type != chunk.r#type || should_emit) {
                            #[cfg(debug_assertions)]
                            {
                                if batch[0].r#type != chunk.r#type {
                                    log::debug!(
                                        "emit_batch: msg_type: {:?}, batch_size: {}, elapsed: {:?}",
                                        chunk.r#type,
                                        batch.len(),
                                        last_emit.elapsed()
                                    );
                                }
                            }

                            // Send current batch
                            if let Err(e) = Self::emit_batch(&window, &batch).await {
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
                });

                tx
            })
            .clone())
    }
}
