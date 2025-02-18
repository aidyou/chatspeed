use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

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
            let window_label_clone = window_label.clone();
            // Spawn a task to handle messages for the window
            tokio::spawn(async move {
                while let Some((chunk, is_error, is_done, is_reasoning, msg_type, metadata)) =
                    rx.recv().await
                {
                    // Emit the event with the received data
                    if let Err(e) = window.emit(
                        "ai_chunk",
                        json!({
                            "chunk": chunk,
                            "is_error": is_error,
                            "is_done": is_done,
                            "is_reasoning": is_reasoning,
                            "window": window_label_clone,
                            "metadata": metadata,
                            "type": msg_type,
                        }),
                    ) {
                        eprintln!("Failed to emit event: {}", e);
                    }
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
