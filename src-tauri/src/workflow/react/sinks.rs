//! Sink implementations for the dispatcher.
//!
//! Sinks are the output targets for dispatched events. Each sink handles
//! a specific type of output (UI, DB, etc.) and operates independently
//! from other sinks.

use std::sync::Arc;

use async_trait::async_trait;

use super::dispatcher::{DispatchEvent, EventEnvelope};
use super::error::WorkflowEngineError;
use super::events::WorkflowEvent;
use super::gateway::Gateway;
use super::types::{ExecutionContext, GatewayPayload};
use crate::db::MainStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkDeliveryGuarantee {
    Reliable,
    BestEffort,
}

/// A sink receives events from the dispatcher and handles them appropriately.
#[async_trait]
pub trait Sink: Send + Sync {
    /// Return whether this sink consumes the event.
    fn accepts(&self, _event: &DispatchEvent) -> bool {
        true
    }

    /// Accept and process an event envelope.
    async fn accept(&self, envelope: EventEnvelope) -> Result<(), WorkflowEngineError>;

    /// Return the name of this sink for logging purposes.
    fn name(&self) -> &str;

    /// Delivery semantics for this sink.
    fn delivery_guarantee(&self) -> SinkDeliveryGuarantee {
        SinkDeliveryGuarantee::BestEffort
    }
}

/// Sink that forwards UI events to the frontend via Tauri gateway.
pub struct TauriSink {
    gateway: Arc<dyn Gateway>,
}

impl TauriSink {
    pub fn new(gateway: Arc<dyn Gateway>) -> Self {
        Self { gateway }
    }
}

#[async_trait]
impl Sink for TauriSink {
    fn accepts(&self, event: &DispatchEvent) -> bool {
        matches!(
            event,
            DispatchEvent::Ui { .. } | DispatchEvent::Terminal { .. }
        )
    }

    async fn accept(&self, envelope: EventEnvelope) -> Result<(), WorkflowEngineError> {
        match envelope.event {
            DispatchEvent::Ui {
                session_id,
                payload,
            } => {
                self.gateway.send(&session_id, payload).await?;
                Ok(())
            }
            DispatchEvent::Terminal { session_id, state } => {
                let mapped_state = match state.as_str() {
                    "completed" => super::types::WorkflowState::Completed,
                    "cancelled" => super::types::WorkflowState::Cancelled,
                    "error" | "failed" => super::types::WorkflowState::Error,
                    _ => super::types::WorkflowState::Error,
                };
                self.gateway
                    .send(
                        &session_id,
                        GatewayPayload::State {
                            state: mapped_state,
                            wait_reason: None,
                        },
                    )
                    .await?;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn name(&self) -> &str {
        "TauriSink"
    }
}

/// Sink that persists events and snapshots to the database.
pub struct DBSink {
    store: Arc<std::sync::RwLock<MainStore>>,
}

impl DBSink {
    pub fn new(store: Arc<std::sync::RwLock<MainStore>>) -> Self {
        Self { store }
    }

    fn append_event_sync(&self, event: &WorkflowEvent) -> Result<i64, WorkflowEngineError> {
        let store = self.store.read().map_err(|e| {
            WorkflowEngineError::Db(crate::db::StoreError::LockError(e.to_string()))
        })?;
        store
            .append_workflow_event(event)
            .map_err(WorkflowEngineError::Db)
    }

    fn upsert_context_sync(&self, ctx: &ExecutionContext) -> Result<(), WorkflowEngineError> {
        let store = self.store.read().map_err(|e| {
            WorkflowEngineError::Db(crate::db::StoreError::LockError(e.to_string()))
        })?;
        store
            .upsert_execution_context(ctx)
            .map_err(WorkflowEngineError::Db)
    }
}

#[async_trait]
impl Sink for DBSink {
    fn accepts(&self, event: &DispatchEvent) -> bool {
        matches!(
            event,
            DispatchEvent::Audit { .. }
                | DispatchEvent::Snapshot { .. }
                | DispatchEvent::Terminal { .. }
        )
    }

    async fn accept(&self, envelope: EventEnvelope) -> Result<(), WorkflowEngineError> {
        match envelope.event {
            DispatchEvent::Audit { event } => {
                self.append_event_sync(&event)?;
                log::info!(
                    "[DBSink] audit event written - session={}, type={:?}",
                    event.session_id,
                    event.event_type
                );
                Ok(())
            }
            DispatchEvent::Snapshot { context } => {
                self.upsert_context_sync(&context)?;
                log::info!(
                    "[DBSink] snapshot written - session={}, state={:?}",
                    context.session_id,
                    context.state
                );
                Ok(())
            }
            DispatchEvent::Terminal { session_id, state } => {
                let event = match state.as_str() {
                    "completed" => {
                        Some(WorkflowEvent::workflow_completed(session_id.clone(), None))
                    }
                    "error" | "failed" => Some(WorkflowEvent::workflow_failed(
                        session_id.clone(),
                        "terminal state".to_string(),
                    )),
                    "cancelled" => Some(WorkflowEvent::workflow_cancelled(session_id.clone())),
                    _ => None,
                };

                if let Some(event) = event {
                    self.append_event_sync(&event)?;
                    log::info!(
                        "[DBSink] terminal event written - session={}, state={}",
                        session_id,
                        state
                    );
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn name(&self) -> &str {
        "DBSink"
    }

    fn delivery_guarantee(&self) -> SinkDeliveryGuarantee {
        SinkDeliveryGuarantee::Reliable
    }
}

/// A sink that can be configured to fail for testing purposes.
#[cfg(test)]
#[allow(dead_code)]
pub struct FailableSink {
    name: String,
    should_fail: std::sync::atomic::AtomicBool,
    call_count: std::sync::atomic::AtomicU64,
}

#[cfg(test)]
#[allow(dead_code)]
impl FailableSink {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            should_fail: std::sync::atomic::AtomicBool::new(false),
            call_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn set_should_fail(&self, value: bool) {
        self.should_fail
            .store(value, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn call_count(&self) -> u64 {
        self.call_count.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(test)]
#[async_trait]
impl Sink for FailableSink {
    async fn accept(&self, _envelope: EventEnvelope) -> Result<(), WorkflowEngineError> {
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if self.should_fail.load(std::sync::atomic::Ordering::Relaxed) {
            Err(WorkflowEngineError::General("sink failure".to_string()))
        } else {
            Ok(())
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// A sink that introduces artificial delay for testing.
#[cfg(test)]
#[allow(dead_code)]
pub struct SlowSink {
    name: String,
    delay_ms: u64,
    call_count: std::sync::atomic::AtomicU64,
}

#[cfg(test)]
#[allow(dead_code)]
impl SlowSink {
    pub fn new(name: &str, delay_ms: u64) -> Self {
        Self {
            name: name.to_string(),
            delay_ms,
            call_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn call_count(&self) -> u64 {
        self.call_count.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(test)]
#[async_trait]
impl Sink for SlowSink {
    async fn accept(&self, _envelope: EventEnvelope) -> Result<(), WorkflowEngineError> {
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        tokio::time::sleep(tokio::time::Duration::from_millis(self.delay_ms)).await;
        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::types::RuntimeState;
    use tempfile::tempdir;

    fn create_test_store() -> Arc<std::sync::RwLock<MainStore>> {
        let dir = tempdir().expect("failed to create temp dir");
        let db_path = dir.path().join("test.db");
        Arc::new(std::sync::RwLock::new(
            MainStore::new(&db_path).expect("failed to create store"),
        ))
    }

    #[tokio::test]
    async fn test_db_sink_writes_audit_events() {
        let store = create_test_store();
        let sink = DBSink::new(store.clone());

        let event =
            WorkflowEvent::workflow_started("test-session".to_string(), "agent-1".to_string());

        let envelope = EventEnvelope::new(DispatchEvent::Audit { event });
        sink.accept(envelope).await.unwrap();

        let store_read = store.read().unwrap();
        let events = store_read.list_workflow_events("test-session").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "workflow_started");
    }

    #[tokio::test]
    async fn test_db_sink_writes_snapshots() {
        let store = create_test_store();
        let sink = DBSink::new(store.clone());

        let mut ctx = ExecutionContext::new("test-session".to_string());
        ctx.state = RuntimeState::Waiting;

        let envelope = EventEnvelope::new(DispatchEvent::Snapshot { context: ctx });
        sink.accept(envelope).await.unwrap();

        let store_read = store.read().unwrap();
        let loaded = store_read.get_execution_context("test-session").unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.state, RuntimeState::Waiting);
    }

    #[tokio::test]
    async fn test_tauri_sink_ignores_audit_events() {
        use std::sync::atomic::AtomicUsize;
        use std::sync::atomic::Ordering;

        struct MockGateway {
            call_count: Arc<AtomicUsize>,
        }

        #[async_trait]
        impl Gateway for MockGateway {
            async fn send(
                &self,
                _session_id: &str,
                _payload: GatewayPayload,
            ) -> Result<(), WorkflowEngineError> {
                self.call_count.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }

            async fn inject_input(
                &self,
                _session_id: &str,
                _input: String,
            ) -> Result<(), WorkflowEngineError> {
                Ok(())
            }
        }

        let call_count = Arc::new(AtomicUsize::new(0));
        let gateway = Arc::new(MockGateway {
            call_count: call_count.clone(),
        });
        let sink = TauriSink::new(gateway);

        let event =
            WorkflowEvent::workflow_started("test-session".to_string(), "agent-1".to_string());
        let envelope = EventEnvelope::new(DispatchEvent::Audit { event });
        sink.accept(envelope).await.unwrap();

        assert_eq!(call_count.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn test_tauri_sink_maps_failed_terminal_to_error_state() {
        use std::sync::atomic::AtomicUsize;
        use std::sync::atomic::Ordering;

        struct MockGateway {
            call_count: Arc<AtomicUsize>,
            last_state: Arc<std::sync::Mutex<Option<super::super::types::WorkflowState>>>,
        }

        #[async_trait]
        impl Gateway for MockGateway {
            async fn send(
                &self,
                _session_id: &str,
                payload: GatewayPayload,
            ) -> Result<(), WorkflowEngineError> {
                self.call_count.fetch_add(1, Ordering::Relaxed);
                if let GatewayPayload::State { state, .. } = payload {
                    let mut guard = self.last_state.lock().unwrap();
                    *guard = Some(state);
                }
                Ok(())
            }

            async fn inject_input(
                &self,
                _session_id: &str,
                _input: String,
            ) -> Result<(), WorkflowEngineError> {
                Ok(())
            }
        }

        let call_count = Arc::new(AtomicUsize::new(0));
        let last_state = Arc::new(std::sync::Mutex::new(None));
        let gateway = Arc::new(MockGateway {
            call_count: call_count.clone(),
            last_state: last_state.clone(),
        });
        let sink = TauriSink::new(gateway);

        let envelope = EventEnvelope::new(DispatchEvent::Terminal {
            session_id: "test-session".to_string(),
            state: "failed".to_string(),
        });
        sink.accept(envelope).await.unwrap();

        assert_eq!(call_count.load(Ordering::Relaxed), 1);
        let state = last_state.lock().unwrap().clone();
        assert_eq!(state, Some(super::super::types::WorkflowState::Error));
    }
}
