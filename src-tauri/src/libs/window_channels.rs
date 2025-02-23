use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration, Instant};

const BATCH_SIZE: usize = 25; // Maximum number of messages to batch
const THROTTLE_DURATION: Duration = Duration::from_millis(20); // ~30fps

// Define a structure to hold the channel sender for each window
pub struct WindowChannels {
    // The parameter is a tuple of (chunk, is_error, is_done,is_reasoning, metadata)
    channels: Arc<
        Mutex<
            HashMap<
                String,
                mpsc::Sender<(String, bool, bool, bool, Option<String>, Option<Value>)>,
            >,
        >,
    >,
}

impl WindowChannels {
    // Create a new instance of WindowChannels
    pub fn new() -> Self {
        Self {
            channels: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Emit an AI chunk message to the specified window
    ///
    /// # Parameters
    /// * `window`: The Tauri window to send the message to
    /// * `chunk`: The text chunk to send
    /// * `is_error`: Whether the chunk is an error
    /// * `is_done`: Whether the chunk is the final chunk for the message
    /// * `is_reasoning`: Whether the chunk is a reasoning step
    /// * `msg_type`: The type of the message (e.g. "chat", "system", "info")
    /// * `metadata`: Additional metadata about the message (e.g. the ID of the message)
    fn emit_ai_chunk(
        window: &tauri::Window,
        chunk: String,
        is_error: bool,
        is_done: bool,
        is_reasoning: bool,
        msg_type: String,
        metadata: Value,
    ) {
        let _ = window
            .emit(
                "ai_chunk",
                json!({
                    "chunk": chunk,
                    "isError": is_error,
                    "isDone": is_done,
                    "isReasoning": is_reasoning,
                    "type": msg_type,
                    "metadata": metadata,
                }),
            )
            .map_err(|e| {
                log::error!("Failed to serialize AI chunk: {}", e);
                e
            });
    }

    /// Emit a batch of AI chunks as a single message
    fn emit_batch(window: &tauri::Window, batch: &[(String, bool, bool, bool, String, Value)]) {
        if batch.is_empty() {
            return;
        }

        let combined_chunk = batch
            .iter()
            .map(|(chunk, ..)| chunk.as_str())
            .collect::<Vec<_>>()
            .join("");

        let first_msg = &batch[0];
        Self::emit_ai_chunk(
            window,
            combined_chunk,
            false,
            false,
            first_msg.3,
            first_msg.4.clone(),
            first_msg.5.clone(),
        );
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
    ) -> Result<mpsc::Sender<(String, bool, bool, bool, Option<String>, Option<Value>)>, String>
    {
        let mut channels = self.channels.lock().await;

        let window_label = window.label().to_string();
        // If the channel for the window is not found, create a new one
        if !channels.contains_key(&window_label) {
            let (tx, mut rx) =
                mpsc::channel::<(String, bool, bool, bool, Option<String>, Option<Value>)>(1000); // Buffer size 1000
                                                                                                  // Spawn a task to handle messages for the window
            tokio::spawn(async move {
                let mut batch = Vec::new();
                let mut last_emit = Instant::now();

                while let Some((chunk, is_error, is_done, is_reasoning, msg_type, metadata)) =
                    rx.recv().await
                {
                    // First message: send immediately
                    if batch.is_empty() {
                        Self::emit_ai_chunk(
                            &window,
                            chunk,
                            is_error,
                            is_done,
                            is_reasoning,
                            msg_type.unwrap_or_default(),
                            metadata.unwrap_or_default(),
                        );

                        // If it's an error or done message, don't start batching
                        if !is_error && !is_done {
                            last_emit = Instant::now();
                        }
                        continue;
                    }

                    // For error or done messages:
                    // 1. Send current batch first
                    // 2. Then send the error/done message separately
                    if is_error || is_done {
                        // Send current batch if not empty
                        Self::emit_batch(&window, &batch);
                        batch.clear();

                        // Send the error/done message
                        Self::emit_ai_chunk(
                            &window,
                            chunk,
                            is_error,
                            is_done,
                            is_reasoning,
                            msg_type.unwrap_or_default(),
                            metadata.unwrap_or_default(),
                        );
                        continue;
                    }

                    // Check if new message is of different type
                    if !batch.is_empty()
                        && (batch[0].3 != is_reasoning
                            || batch[0].4 != msg_type.clone().unwrap_or_default())
                    {
                        // Send current batch
                        Self::emit_batch(&window, &batch);
                        batch.clear();
                    }

                    // Add new message to batch
                    batch.push((
                        chunk,
                        is_error,
                        is_done,
                        is_reasoning,
                        msg_type.unwrap_or_default(),
                        metadata.unwrap_or_default(),
                    ));

                    // Check if we should emit based on batch size or time
                    let should_emit =
                        batch.len() >= BATCH_SIZE || last_emit.elapsed() >= THROTTLE_DURATION;

                    if should_emit {
                        Self::emit_batch(&window, &batch);
                        batch.clear();
                        last_emit = Instant::now();
                    }

                    // Add a small delay to prevent CPU overload
                    sleep(Duration::from_millis(1)).await;
                }
            });

            channels.insert(window_label, tx.clone());
            return Ok(tx);
        } else {
            // Return the existing channel for the window
            if let Some(tx) = channels.get(&window_label) {
                return Ok(tx.clone());
            } else {
                Err("Channel not found".to_string())
            }
        }
    }
}
