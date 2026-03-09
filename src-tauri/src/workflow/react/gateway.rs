use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::types::GatewayPayload;

use async_trait::async_trait;
use std::collections::HashMap;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration, Instant};

#[async_trait]
pub trait Gateway: Send + Sync {
    /// Sends an event to the external world (e.g., UI, WebSocket)
    async fn send(
        &self,
        session_id: &str,
        payload: GatewayPayload,
    ) -> Result<(), WorkflowEngineError>;

    /// This should be called by a Tauri command to inject user input or approvals
    async fn inject_input(
        &self,
        session_id: &str,
        input: String,
    ) -> Result<(), WorkflowEngineError>;

    /// Registers a sender for a specific session to receive external signals
    async fn register_session_tx(&self, session_id: String, tx: mpsc::Sender<String>);
}

/// A Gateway implementation specifically for Tauri with throttling and batching
pub struct TauriGateway {
    app_handle: AppHandle,
    /// Map of session_id -> mpsc sender for inputs.
    input_senders: Mutex<HashMap<String, mpsc::Sender<String>>>,
    /// Map of session_id -> mpsc sender for aggregated events.
    event_senders: Mutex<HashMap<String, mpsc::Sender<GatewayPayload>>>,
}

impl TauriGateway {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            app_handle,
            input_senders: Mutex::new(HashMap::new()),
            event_senders: Mutex::new(HashMap::new()),
        }
    }

    /// Create or get an aggregated channel for a session
    async fn get_or_create_event_channel(&self, session_id: &str) -> mpsc::Sender<GatewayPayload> {
        let mut senders = self.event_senders.lock().await;
        if let Some(tx) = senders.get(session_id) {
            return tx.clone();
        }

        let (tx, mut rx) = mpsc::channel::<GatewayPayload>(1000);
        let app_handle = self.app_handle.clone();
        let event_name = format!("workflow://event/{}", session_id);

        tokio::spawn(async move {
            let mut chunk_buffer = String::new();
            let mut reasoning_buffer = String::new();
            let mut last_emit = Instant::now();
            let throttle_duration = Duration::from_millis(100);
            
            // Tracking code block state to avoid partial high-load re-renders
            let mut in_code_block = false;

            while let Some(payload) = rx.recv().await {
                match payload {
                    GatewayPayload::Chunk { content } => {
                        // Detect code block toggles
                        // Note: This is a simple count-based detection for stream batching
                        let backtick_count = content.matches("```").count();
                        if backtick_count % 2 != 0 {
                            in_code_block = !in_code_block;
                        }
                        
                        chunk_buffer.push_str(&content);
                        
                        // If we are in a code block, we can batch more aggressively
                        // If not, or if duration reached, emit
                        let should_emit = !in_code_block && (last_emit.elapsed() >= throttle_duration || chunk_buffer.len() > 500);
                        
                        if should_emit {
                            let _ = app_handle.emit(&event_name, GatewayPayload::Chunk { content: chunk_buffer.clone() });
                            chunk_buffer.clear();
                            last_emit = Instant::now();
                        }
                    }
                    GatewayPayload::ReasoningChunk { content } => {
                        reasoning_buffer.push_str(&content);
                        if last_emit.elapsed() >= throttle_duration {
                            let _ = app_handle.emit(&event_name, GatewayPayload::ReasoningChunk { content: reasoning_buffer.clone() });
                            reasoning_buffer.clear();
                            last_emit = Instant::now();
                        }
                    }
                    // Non-chunk messages (State, Confirm, Full Messages) should flush buffers and be sent immediately
                    _ => {
                        // Flush text buffer
                        if !chunk_buffer.is_empty() {
                            let _ = app_handle.emit(&event_name, GatewayPayload::Chunk { content: chunk_buffer.clone() });
                            chunk_buffer.clear();
                        }
                        // Flush reasoning buffer
                        if !reasoning_buffer.is_empty() {
                            let _ = app_handle.emit(&event_name, GatewayPayload::ReasoningChunk { content: reasoning_buffer.clone() });
                            reasoning_buffer.clear();
                        }
                        
                        // Send the control message immediately
                        let _ = app_handle.emit(&event_name, &payload);
                        last_emit = Instant::now();
                    }
                }
                
                // Small sleep to yield
                sleep(Duration::from_millis(5)).await;
            }
        });

        senders.insert(session_id.to_string(), tx.clone());
        tx
    }
}

#[async_trait]
impl Gateway for TauriGateway {
    async fn send(
        &self,
        session_id: &str,
        payload: GatewayPayload,
    ) -> Result<(), WorkflowEngineError> {
        let tx = self.get_or_create_event_channel(session_id).await;
        tx.send(payload).await.map_err(|e| WorkflowEngineError::Gateway(e.to_string()))?;
        Ok(())
    }

    async fn inject_input(
        &self,
        session_id: &str,
        input: String,
    ) -> Result<(), WorkflowEngineError> {
        log::debug!(
            "[Workflow Gateway] Injecting input for session {}: {}",
            session_id,
            input
        );
        let senders = self.input_senders.lock().await;
        if let Some(tx) = senders.get(session_id) {
            tx.send(input)
                .await
                .map_err(|e| WorkflowEngineError::Gateway(e.to_string()))?;
            Ok(())
        } else {
            log::warn!(
                "[Workflow Gateway] Failed to inject input: No sender for session {}",
                session_id
            );
            Err(WorkflowEngineError::Gateway(format!(
                "No input channel for session {}",
                session_id
            )))
        }
    }

    async fn register_session_tx(&self, session_id: String, tx: mpsc::Sender<String>) {
        log::debug!(
            "[Workflow Gateway] Registering input sender for session {}",
            session_id
        );
        let mut senders = self.input_senders.lock().await;
        senders.insert(session_id, tx);
    }
}
