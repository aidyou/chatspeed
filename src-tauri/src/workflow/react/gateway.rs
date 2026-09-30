use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::types::GatewayPayload;

use async_trait::async_trait;
use tokio::sync::mpsc;

#[async_trait]
pub trait Gateway: Send + Sync {
    /// Sends an event to the external world (e.g., UI, WebSocket)
    async fn send(
        &self,
        session_id: &str,
        payload: GatewayPayload,
    ) -> Result<(), WorkflowEngineError>;

    /// Registers the input channel owned by a live workflow session.
    async fn register_session_input(
        &self,
        _session_id: String,
        _tx: mpsc::Sender<String>,
    ) -> Result<(), WorkflowEngineError> {
        Ok(())
    }

    /// Removes all transport channels owned by a workflow session.
    async fn unregister_session_input(&self, _session_id: &str) {}

    /// This should be called by a Tauri command to inject user input or approvals
    async fn inject_input(
        &self,
        session_id: &str,
        input: String,
    ) -> Result<(), WorkflowEngineError>;
}
