use async_trait::async_trait;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use crate::workflow::react::types::GatewayPayload;
use crate::workflow::react::error::WorkflowEngineError;

#[async_trait]
pub trait Gateway: Send + Sync {
    /// Sends an event to the external world (e.g., UI, WebSocket)
    async fn send(&self, session_id: &str, payload: GatewayPayload) -> Result<(), WorkflowEngineError>;

    /// Receives a signal from the external world (e.g., user input, approval)
    async fn receive_input(&self, session_id: &str) -> Result<String, WorkflowEngineError>;
}

/// A Gateway implementation specifically for Tauri
pub struct TauriGateway {
    app_handle: AppHandle,
    /// Map of session_id -> mpsc sender for inputs. 
    /// We use a Mutex of a HashMap of Senders.
    input_senders: Mutex<HashMap<String, tokio::sync::mpsc::Sender<String>>>,
}

impl TauriGateway {
    pub fn new(app_handle: AppHandle) -> Self {
        Self { 
            app_handle,
            input_senders: Mutex::new(HashMap::new()),
        }
    }

    /// Creates a new input channel for a session and returns the receiver
    pub async fn create_session_channel(&self, session_id: &str) -> tokio::sync::mpsc::Receiver<String> {
        let (tx, rx) = tokio::sync::mpsc::channel(10);
        let mut senders = self.input_senders.lock().await;
        senders.insert(session_id.to_string(), tx);
        rx
    }

    /// This should be called by a Tauri command to inject user input
    pub async fn inject_input(&self, session_id: &str, input: String) -> Result<(), WorkflowEngineError> {
        let senders = self.input_senders.lock().await;
        if let Some(tx) = senders.get(session_id) {
            tx.send(input).await.map_err(|e| WorkflowEngineError::Gateway(e.to_string()))?;
            Ok(())
        } else {
            Err(WorkflowEngineError::Gateway(format!("No input channel for session {}", session_id)))
        }
    }
}

#[async_trait]
impl Gateway for TauriGateway {
    async fn send(&self, session_id: &str, payload: GatewayPayload) -> Result<(), WorkflowEngineError> {
        let event_name = format!("workflow://event/{}", session_id);
        self.app_handle.emit(&event_name, &payload)
            .map_err(|e| WorkflowEngineError::Gateway(e.to_string()))?;
        Ok(())
    }

    async fn receive_input(&self, session_id: &str) -> Result<String, WorkflowEngineError> {
        let mut rx = self.create_session_channel(session_id).await;
        rx.recv().await.ok_or_else(|| WorkflowEngineError::Gateway("Input channel closed".to_string()))
    }
}
