use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration, Instant};

use crate::ai::traits::chat::ChatResponse;

const BATCH_SIZE: usize = 200; // Maximum number of messages to batch
const THROTTLE_DURATION: Duration = Duration::from_millis(200); // ~200fps

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

    /// Emit an AI chunk message to the specified window
    ///
    /// # Parameters
    /// * `window`: The Tauri window to send the message to
    /// * `chunk`: message to send
    async fn emit_ai_chunk(
        window: &tauri::Window,
        chunk: Arc<ChatResponse>,
    ) -> Result<(), tauri::Error> {
        window.emit("ai_chunk", json!(&chunk)).map_err(|e| {
            log::error!("Failed to serialize AI chunk: {}", e);
            e
        })
    }

    /// Emit a batch of AI chunks as a single message
    ///
    /// # Parameters
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
        Self::emit_ai_chunk(
            window,
            Arc::new(ChatResponse {
                chat_id: first_msg.chat_id.clone(),
                chunk: combined,
                is_error: false,
                is_done: false,
                is_reasoning: first_msg.is_reasoning,
                metadata: first_msg.metadata.clone(),
                r#type: first_msg.r#type.clone(),
            }),
        )
        .await
    }

    /// Get or create a channel for the specified window.
    ///
    /// # Parameters
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
                    let mut is_first: bool = true;
                    let mut batch = Vec::<ChatResponse>::with_capacity(BATCH_SIZE as usize);
                    let mut last_emit = Instant::now();

                    while let Some(chunk) = rx.recv().await {
                        // First message: send immediately
                        if is_first && !chunk.chunk.is_empty() {
                            is_first = false;

                            #[cfg(debug_assertions)]
                            log::debug!("Sending first message: {:?}", chunk);

                            if let Err(e) = Self::emit_ai_chunk(&window, chunk.clone()).await {
                                log::error!("Failed to emit AI chunk: {}", e);
                            }

                            // If it's an error or done message, don't start batching
                            if !chunk.is_error && !chunk.is_done {
                                last_emit = Instant::now();
                            }
                            continue;
                        }

                        // For error or done messages:
                        // 1. Send current batch first
                        // 2. Then send the error/done message separately
                        if chunk.is_error || chunk.is_done {
                            #[cfg(debug_assertions)]
                            log::debug!(
                                "sending error or done, is_error: {:?}, is_done: {:?}",
                                chunk.is_error,
                                chunk.is_done
                            );

                            // Send current batch if not empty
                            if let Err(e) = Self::emit_batch(&window, &batch).await {
                                log::error!("Failed to emit batch: {}", e);
                            }
                            batch.clear();

                            // Send the error/done message
                            if let Err(e) = Self::emit_ai_chunk(&window, chunk).await {
                                log::error!("Failed to emit AI chunk: {}", e);
                            }
                            continue;
                        }

                        // Check if new message is of different type
                        if !batch.is_empty()
                            && (batch[0].is_reasoning != chunk.is_reasoning
                                || batch[0].r#type != chunk.r#type)
                        {
                            #[cfg(debug_assertions)]
                            log::debug!(
                                "emit_batch: is_reasoning: {:?}, msg_type: {:?}",
                                chunk.is_reasoning,
                                chunk.r#type.clone().unwrap_or_default()
                            );

                            // Send current batch
                            if let Err(e) = Self::emit_batch(&window, &batch).await {
                                log::error!("Failed to emit batch: {}", e);
                            }
                            batch.clear();
                        }

                        // Add new message to batch
                        batch.push(chunk.as_ref().clone());

                        // Check if we should emit based on batch size or time
                        let should_emit =
                            batch.len() >= BATCH_SIZE || last_emit.elapsed() >= THROTTLE_DURATION;

                        if should_emit {
                            #[cfg(debug_assertions)]
                            log::debug!(
                                "should_emit: batch.len(): {:?}, THROTTLE_DURATION: {:?}",
                                batch.len(),
                                THROTTLE_DURATION
                            );

                            if let Err(e) = Self::emit_batch(&window, &batch).await {
                                log::error!("Failed to emit batch: {}", e);
                            }
                            batch.clear();
                            last_emit = Instant::now();
                        }

                        // Add a small delay to prevent CPU overload
                        sleep(Duration::from_millis(1)).await;
                    }
                });

                tx
            })
            .clone())
    }
}
