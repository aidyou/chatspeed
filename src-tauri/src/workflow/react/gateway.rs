use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::types::GatewayPayload;

use async_trait::async_trait;
use std::collections::HashMap;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

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
    async fn register_session_tx(&self, session_id: String, tx: tokio::sync::mpsc::Sender<String>);
}

/// A Gateway implementation specifically for Tauri
pub struct TauriGateway {
    app_handle: AppHandle,
    /// Map of session_id -> mpsc sender for inputs.
    input_senders: Mutex<HashMap<String, tokio::sync::mpsc::Sender<String>>>,
}

impl TauriGateway {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            app_handle,
            input_senders: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl Gateway for TauriGateway {
    async fn send(
        &self,
        session_id: &str,
        payload: GatewayPayload,
    ) -> Result<(), WorkflowEngineError> {
        let event_name = format!("workflow://event/{}", session_id);
        log::debug!(
            "[Workflow Gateway] Emitting event to {}: {:?}",
            event_name,
            payload
        );
        self.app_handle
            .emit(&event_name, &payload)
            .map_err(|e| WorkflowEngineError::Gateway(e.to_string()))?;
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

    async fn register_session_tx(&self, session_id: String, tx: tokio::sync::mpsc::Sender<String>) {
        log::debug!(
            "[Workflow Gateway] Registering input sender for session {}",
            session_id
        );
        let mut senders = self.input_senders.lock().await;
        senders.insert(session_id, tx);
    }
}
