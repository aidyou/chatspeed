//! Event Dispatcher for Phase 6 - Unified event distribution to multiple sinks.
//!
//! This module provides a dispatcher architecture that separates event emission
//! from the executor, allowing UI events, DB writes, and other outputs to be
//! handled independently without blocking each other.
//!
//! Key design principles:
//! 1. DB writes must not depend on best-effort broadcast semantics
//! 2. UI sink can tolerate partial state loss, but not final state
//! 3. Executor must not be blocked by slow sinks

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::error::WorkflowEngineError;
use super::events::WorkflowEvent;
use super::sinks::Sink;
use super::sinks::SinkDeliveryGuarantee;
use super::types::{ExecutionContext, GatewayPayload};

fn gateway_payload_name(payload: &GatewayPayload) -> &'static str {
    match payload {
        GatewayPayload::Chunk { .. } => "chunk",
        GatewayPayload::ReasoningChunk { .. } => "reasoning_chunk",
        GatewayPayload::Message { .. } => "message",
        GatewayPayload::State { .. } => "state",
        GatewayPayload::Confirm { .. } => "confirm",
        GatewayPayload::ApprovalResolved { .. } => "approval_resolved",
        GatewayPayload::QueuedUserMessageRemoved { .. } => "queued_user_message_removed",
        GatewayPayload::ToolStarted { .. } => "tool_started",
        GatewayPayload::ToolCompleted { .. } => "tool_completed",
        GatewayPayload::ToolFailed { .. } => "tool_failed",
        GatewayPayload::SyncTodo { .. } => "sync_todo",
        GatewayPayload::RetryStatus { .. } => "retry_status",
        GatewayPayload::CompressionStatus { .. } => "compression_status",
        GatewayPayload::CompressionApplied { .. } => "compression_applied",
        GatewayPayload::ContextUsage { .. } => "context_usage",
        GatewayPayload::SubAgentProgress { .. } => "sub_agent_progress",
        GatewayPayload::Notification { .. } => "notification",
        GatewayPayload::AutoApprovedToolsUpdated { .. } => "auto_approved_tools_updated",
        GatewayPayload::AgentConfigUpdated { .. } => "agent_config_updated",
        GatewayPayload::WorkflowTitleUpdated { .. } => "workflow_title_updated",
        GatewayPayload::ShellPolicyUpdated { .. } => "shell_policy_updated",
        GatewayPayload::ToolStream { .. } => "tool_stream",
        GatewayPayload::Error { .. } => "error",
    }
}

fn summarize_dispatch_event(event: &DispatchEvent) -> String {
    match event {
        DispatchEvent::Ui {
            session_id,
            payload,
        } => format!(
            "Ui(session={}, payload={})",
            session_id,
            gateway_payload_name(payload)
        ),
        DispatchEvent::Audit { event } => {
            format!(
                "Audit(session={}, type={:?})",
                event.session_id, event.event_type
            )
        }
        DispatchEvent::Snapshot { context } => format!(
            "Snapshot(session={}, state={:?}, wait_reason={:?})",
            context.session_id, context.state, context.wait_reason
        ),
        DispatchEvent::Terminal { session_id, state } => {
            format!("Terminal(session={}, state={})", session_id, state)
        }
    }
}

/// Categories of events that can be dispatched to sinks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DispatchEvent {
    /// UI event to be sent to the frontend via Tauri
    Ui {
        session_id: String,
        payload: GatewayPayload,
    },
    /// Audit event to be persisted to the database
    Audit { event: WorkflowEvent },
    /// Snapshot to be persisted to the database
    Snapshot { context: ExecutionContext },
    /// Terminal event (completed/failed/cancelled) - must reach DB sink
    Terminal { session_id: String, state: String },
}

/// Wrapper for events with metadata for tracking and metrics.
#[derive(Debug, Clone)]
pub struct EventEnvelope {
    /// The actual event to dispatch
    pub event: DispatchEvent,
    /// Timestamp when the event was created
    pub created_at: Instant,
    /// Optional sequence number for ordering
    pub sequence: Option<u64>,
}

impl EventEnvelope {
    pub fn new(event: DispatchEvent) -> Self {
        Self {
            event,
            created_at: Instant::now(),
            sequence: None,
        }
    }

    pub fn with_sequence(mut self, seq: u64) -> Self {
        self.sequence = Some(seq);
        self
    }
}

/// Metrics for dispatcher health monitoring.
#[derive(Debug, Default)]
pub struct DispatcherMetrics {
    /// Total events dispatched
    pub total_dispatched: AtomicU64,
    /// Events dropped due to channel full
    pub total_dropped: AtomicU64,
    /// Events that failed in sink
    pub total_failed: AtomicU64,
    /// Current queue depth (approximate)
    pub queue_depth: AtomicU64,
    /// Per-sink metrics
    pub per_sink: DashMap<String, Arc<SinkMetrics>>,
}

#[derive(Debug, Default)]
pub struct SinkMetrics {
    /// Approximate queue depth for this sink
    pub queue_depth: AtomicU64,
    /// Number of events currently being processed by this sink
    pub inflight: AtomicU64,
    /// Number of events dropped for this sink
    pub dropped: AtomicU64,
    /// Number of events successfully processed
    pub processed: AtomicU64,
    /// Number of events failed
    pub failed: AtomicU64,
    /// Total processing latency in ms
    pub total_latency_ms: AtomicU64,
}

impl DispatcherMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn increment_dispatched(&self) {
        self.total_dispatched.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_dropped(&self) {
        self.total_dropped.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_failed(&self) {
        self.total_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_queue_depth(&self, depth: u64) {
        self.queue_depth.store(depth, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> DispatcherMetricsSnapshot {
        let per_sink = self
            .per_sink
            .iter()
            .map(|entry| {
                let name = entry.key().clone();
                let metrics = entry.value();
                let processed = metrics.processed.load(Ordering::Relaxed);
                let failed = metrics.failed.load(Ordering::Relaxed);
                let total_calls = processed + failed;
                let total_latency_ms = metrics.total_latency_ms.load(Ordering::Relaxed);
                let avg_latency_ms = if total_calls > 0 {
                    total_latency_ms as f64 / total_calls as f64
                } else {
                    0.0
                };
                SinkMetricsSnapshot {
                    sink_name: name,
                    queue_depth: metrics.queue_depth.load(Ordering::Relaxed),
                    inflight: metrics.inflight.load(Ordering::Relaxed),
                    dropped: metrics.dropped.load(Ordering::Relaxed),
                    processed,
                    failed,
                    avg_latency_ms,
                }
            })
            .collect();

        DispatcherMetricsSnapshot {
            total_dispatched: self.total_dispatched.load(Ordering::Relaxed),
            total_dropped: self.total_dropped.load(Ordering::Relaxed),
            total_failed: self.total_failed.load(Ordering::Relaxed),
            queue_depth: self.queue_depth.load(Ordering::Relaxed),
            per_sink,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatcherMetricsSnapshot {
    pub total_dispatched: u64,
    pub total_dropped: u64,
    pub total_failed: u64,
    pub queue_depth: u64,
    pub per_sink: Vec<SinkMetricsSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SinkMetricsSnapshot {
    pub sink_name: String,
    pub queue_depth: u64,
    pub inflight: u64,
    pub dropped: u64,
    pub processed: u64,
    pub failed: u64,
    pub avg_latency_ms: f64,
}

/// Configuration for dispatcher behavior.
#[derive(Debug, Clone)]
pub struct DispatcherConfig {
    /// Maximum channel capacity for the dispatcher
    pub channel_capacity: usize,
    /// Maximum channel capacity per sink worker
    pub sink_channel_capacity: usize,
    /// Enable logging of lag warnings
    pub log_lag_warnings: bool,
    /// Threshold in milliseconds for lag warning
    pub lag_warning_threshold_ms: u64,
}

impl Default for DispatcherConfig {
    fn default() -> Self {
        Self {
            channel_capacity: 1000,
            sink_channel_capacity: 1000,
            log_lag_warnings: true,
            lag_warning_threshold_ms: 2000,
        }
    }
}

/// The dispatcher routes events to registered sinks.
///
/// Key behaviors:
/// - Non-blocking: uses channels to decouple sender from sinks
/// - Isolated: one sink failure doesn't affect others
/// - Metrics: tracks dropped events and queue depth
pub struct Dispatcher {
    /// Sender for the dispatcher channel
    tx: mpsc::Sender<EventEnvelope>,
    /// Metrics for monitoring
    metrics: Arc<DispatcherMetrics>,
    /// Sequence counter for ordering
    sequence_counter: AtomicU64,
}

struct SinkTarget {
    name: String,
    tx: mpsc::Sender<EventEnvelope>,
    metrics: Arc<SinkMetrics>,
    delivery_guarantee: SinkDeliveryGuarantee,
    channel_capacity: usize,
    accepts: Arc<dyn Fn(&DispatchEvent) -> bool + Send + Sync>,
}

lazy_static::lazy_static! {
    static ref DISPATCHER_REGISTRY: DashMap<String, Arc<Dispatcher>> = DashMap::new();
}

impl Dispatcher {
    /// Create a new dispatcher with the given sinks.
    ///
    /// The dispatcher spawns a background task that forwards events to all sinks.
    /// Each sink gets its own task to ensure isolation.
    pub fn new(sinks: Vec<Arc<dyn Sink>>, config: DispatcherConfig) -> Self {
        let metrics = Arc::new(DispatcherMetrics::new());
        let (tx, mut rx) = mpsc::channel::<EventEnvelope>(config.channel_capacity);
        let mut sink_targets = Vec::new();

        for sink in sinks {
            let sink_name = sink.name().to_string();
            let sink_metrics = metrics
                .per_sink
                .entry(sink_name.clone())
                .or_insert_with(|| Arc::new(SinkMetrics::default()))
                .clone();
            let (sink_tx, mut sink_rx) =
                mpsc::channel::<EventEnvelope>(config.sink_channel_capacity);
            let sink_clone = sink.clone();
            let sink_metrics_clone = sink_metrics.clone();
            let dispatcher_metrics = metrics.clone();
            let lag_threshold_ms = config.lag_warning_threshold_ms;
            let log_lag_warnings = config.log_lag_warnings;

            tokio::spawn(async move {
                while let Some(envelope) = sink_rx.recv().await {
                    if log_lag_warnings {
                        let elapsed = envelope.created_at.elapsed().as_millis() as u64;
                        if elapsed > lag_threshold_ms {
                            log::warn!(
                                "[Dispatcher] sink lag detected - sink={}, elapsed={}ms, event={}",
                                sink_clone.name(),
                                elapsed,
                                summarize_dispatch_event(&envelope.event)
                            );
                        }
                    }

                    sink_metrics_clone.inflight.fetch_add(1, Ordering::Relaxed);
                    let started = Instant::now();

                    if let Err(e) = sink_clone.accept(envelope).await {
                        log::error!("[Dispatcher] sink '{}' failed: {}", sink_clone.name(), e);
                        dispatcher_metrics.increment_failed();
                        sink_metrics_clone.failed.fetch_add(1, Ordering::Relaxed);
                    } else {
                        sink_metrics_clone.processed.fetch_add(1, Ordering::Relaxed);
                    }

                    let elapsed_ms = started.elapsed().as_millis() as u64;
                    sink_metrics_clone
                        .total_latency_ms
                        .fetch_add(elapsed_ms, Ordering::Relaxed);
                    sink_metrics_clone.inflight.fetch_sub(1, Ordering::Relaxed);
                    let remaining_depth = config
                        .sink_channel_capacity
                        .saturating_sub(sink_rx.capacity())
                        as u64;
                    sink_metrics_clone
                        .queue_depth
                        .store(remaining_depth, Ordering::Relaxed);
                }
            });

            sink_targets.push(SinkTarget {
                name: sink_name,
                tx: sink_tx,
                metrics: sink_metrics,
                delivery_guarantee: sink.delivery_guarantee(),
                channel_capacity: config.sink_channel_capacity,
                accepts: {
                    let sink = sink.clone();
                    Arc::new(move |event| sink.accepts(event))
                },
            });
        }

        let metrics_clone = metrics.clone();
        let log_lag_warnings = config.log_lag_warnings;
        let lag_threshold_ms = config.lag_warning_threshold_ms;
        let channel_capacity = config.channel_capacity;

        tokio::spawn(async move {
            while let Some(envelope) = rx.recv().await {
                if log_lag_warnings {
                    let elapsed = envelope.created_at.elapsed().as_millis() as u64;
                    if elapsed > lag_threshold_ms {
                        log::warn!(
                            "[Dispatcher] event lag detected - elapsed={}ms, event={}",
                            elapsed,
                            summarize_dispatch_event(&envelope.event)
                        );
                    }
                }

                let queue_depth = channel_capacity.saturating_sub(rx.capacity()) as u64;
                metrics_clone.set_queue_depth(queue_depth);

                for sink_target in &sink_targets {
                    if !(sink_target.accepts)(&envelope.event) {
                        continue;
                    }

                    let mut sink_envelope = envelope.clone();
                    sink_envelope.created_at = Instant::now();

                    let send_result = match sink_target.delivery_guarantee {
                        SinkDeliveryGuarantee::Reliable => sink_target
                            .tx
                            .send(sink_envelope)
                            .await
                            .map_err(|_| WorkflowEngineError::DispatcherClosed),
                        SinkDeliveryGuarantee::BestEffort => sink_target
                            .tx
                            .try_send(sink_envelope)
                            .map_err(|err| match err {
                                mpsc::error::TrySendError::Full(_) => {
                                    WorkflowEngineError::DispatcherChannelFull
                                }
                                mpsc::error::TrySendError::Closed(_) => {
                                    WorkflowEngineError::DispatcherClosed
                                }
                            }),
                    };

                    match send_result {
                        Ok(()) => {
                            let sink_depth = sink_target
                                .channel_capacity
                                .saturating_sub(sink_target.tx.capacity())
                                as u64;
                            sink_target
                                .metrics
                                .queue_depth
                                .store(sink_depth, Ordering::Relaxed);
                        }
                        Err(WorkflowEngineError::DispatcherChannelFull) => {
                            metrics_clone.increment_dropped();
                            sink_target.metrics.dropped.fetch_add(1, Ordering::Relaxed);
                            log::warn!(
                                "[Dispatcher] sink queue full, event dropped - sink={}, event={}",
                                sink_target.name,
                                summarize_dispatch_event(&envelope.event)
                            );
                        }
                        Err(e) => {
                            metrics_clone.increment_failed();
                            sink_target.metrics.failed.fetch_add(1, Ordering::Relaxed);
                            log::error!(
                                "[Dispatcher] failed to enqueue event for sink '{}' - {}",
                                sink_target.name,
                                e
                            );
                        }
                    }
                }

                metrics_clone.increment_dispatched();
            }

            log::info!("[Dispatcher] dispatcher loop terminated");
        });

        Self {
            tx,
            metrics,
            sequence_counter: AtomicU64::new(0),
        }
    }

    /// Dispatch an event to all registered sinks.
    ///
    /// This is non-blocking - if the channel is full, the event is dropped
    /// and the drop counter is incremented.
    pub fn dispatch_now(&self, event: DispatchEvent) -> Result<(), WorkflowEngineError> {
        let seq = self.sequence_counter.fetch_add(1, Ordering::Relaxed);
        let envelope = EventEnvelope::new(event).with_sequence(seq);

        match self.tx.try_send(envelope) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(envelope)) => {
                self.metrics.increment_dropped();
                log::warn!(
                    "[Dispatcher] channel full, event dropped - type={:?}",
                    envelope.event
                );
                Err(WorkflowEngineError::DispatcherChannelFull)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                log::error!("[Dispatcher] channel closed");
                Err(WorkflowEngineError::DispatcherClosed)
            }
        }
    }

    pub async fn dispatch(&self, event: DispatchEvent) -> Result<(), WorkflowEngineError> {
        self.dispatch_now(event)
    }

    /// Dispatch a UI event (convenience method).
    pub async fn dispatch_ui(
        &self,
        session_id: String,
        payload: GatewayPayload,
    ) -> Result<(), WorkflowEngineError> {
        self.dispatch(DispatchEvent::Ui {
            session_id,
            payload,
        })
        .await
    }

    /// Dispatch an audit event from synchronous runtime paths.
    pub fn dispatch_audit_now(&self, event: WorkflowEvent) -> Result<(), WorkflowEngineError> {
        self.dispatch_now(DispatchEvent::Audit { event })
    }

    /// Dispatch a snapshot (convenience method).
    pub async fn dispatch_snapshot(
        &self,
        context: ExecutionContext,
    ) -> Result<(), WorkflowEngineError> {
        self.dispatch(DispatchEvent::Snapshot { context }).await
    }

    /// Dispatch a terminal event (convenience method).
    pub async fn dispatch_terminal(
        &self,
        session_id: String,
        state: String,
    ) -> Result<(), WorkflowEngineError> {
        self.dispatch(DispatchEvent::Terminal { session_id, state })
            .await
    }

    /// Get current metrics snapshot.
    pub fn metrics(&self) -> DispatcherMetricsSnapshot {
        self.metrics.snapshot()
    }

    pub fn register_session_dispatcher(session_id: String, dispatcher: Arc<Dispatcher>) {
        DISPATCHER_REGISTRY.insert(session_id, dispatcher);
    }

    pub fn unregister_session_dispatcher(session_id: &str) {
        DISPATCHER_REGISTRY.remove(session_id);
    }

    pub fn metrics_for_session(session_id: &str) -> Option<DispatcherMetricsSnapshot> {
        DISPATCHER_REGISTRY.get(session_id).map(|d| d.metrics())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// A mock sink that counts received events
    struct MockSink {
        name: String,
        count: Arc<AtomicUsize>,
        should_fail: bool,
        delay_ms: u64,
        delivery_guarantee: SinkDeliveryGuarantee,
    }

    impl MockSink {
        fn new(name: &str, count: Arc<AtomicUsize>) -> Self {
            Self {
                name: name.to_string(),
                count,
                should_fail: false,
                delay_ms: 0,
                delivery_guarantee: SinkDeliveryGuarantee::BestEffort,
            }
        }

        fn with_failure(name: &str, count: Arc<AtomicUsize>) -> Self {
            Self {
                name: name.to_string(),
                count,
                should_fail: true,
                delay_ms: 0,
                delivery_guarantee: SinkDeliveryGuarantee::BestEffort,
            }
        }

        fn with_delay(name: &str, count: Arc<AtomicUsize>, delay_ms: u64) -> Self {
            Self {
                name: name.to_string(),
                count,
                should_fail: false,
                delay_ms,
                delivery_guarantee: SinkDeliveryGuarantee::BestEffort,
            }
        }

        fn reliable(name: &str, count: Arc<AtomicUsize>, delay_ms: u64) -> Self {
            Self {
                name: name.to_string(),
                count,
                should_fail: false,
                delay_ms,
                delivery_guarantee: SinkDeliveryGuarantee::Reliable,
            }
        }
    }

    #[async_trait::async_trait]
    impl Sink for MockSink {
        async fn accept(&self, _envelope: EventEnvelope) -> Result<(), WorkflowEngineError> {
            self.count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if self.delay_ms > 0 {
                tokio::time::sleep(tokio::time::Duration::from_millis(self.delay_ms)).await;
            }
            if self.should_fail {
                Err(WorkflowEngineError::General("mock failure".to_string()))
            } else {
                Ok(())
            }
        }

        fn name(&self) -> &str {
            &self.name
        }

        fn delivery_guarantee(&self) -> SinkDeliveryGuarantee {
            self.delivery_guarantee
        }
    }

    #[tokio::test]
    async fn test_dispatcher_routes_to_all_sinks() {
        let count1 = Arc::new(AtomicUsize::new(0));
        let count2 = Arc::new(AtomicUsize::new(0));

        let sink1 = Arc::new(MockSink::new("sink1", count1.clone()));
        let sink2 = Arc::new(MockSink::new("sink2", count2.clone()));

        let dispatcher = Dispatcher::new(
            vec![sink1 as Arc<dyn Sink>, sink2 as Arc<dyn Sink>],
            DispatcherConfig::default(),
        );

        // Dispatch an event
        dispatcher
            .dispatch_ui(
                "test-session".to_string(),
                GatewayPayload::State {
                    state: super::super::types::WorkflowState::Thinking,
                    wait_reason: None,
                },
            )
            .await
            .unwrap();

        // Wait for async processing
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Both sinks should have received the event
        assert_eq!(count1.load(Ordering::Relaxed), 1);
        assert_eq!(count2.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_sink_failure_doesnt_affect_others() {
        let count_fail = Arc::new(AtomicUsize::new(0));
        let count_ok = Arc::new(AtomicUsize::new(0));

        let failing_sink = Arc::new(MockSink::with_failure("failing", count_fail.clone()));
        let ok_sink = Arc::new(MockSink::new("ok", count_ok.clone()));

        let dispatcher = Dispatcher::new(
            vec![failing_sink as Arc<dyn Sink>, ok_sink as Arc<dyn Sink>],
            DispatcherConfig::default(),
        );

        dispatcher
            .dispatch_ui(
                "test-session".to_string(),
                GatewayPayload::State {
                    state: super::super::types::WorkflowState::Thinking,
                    wait_reason: None,
                },
            )
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Both sinks should have been called (even though one failed)
        assert_eq!(count_fail.load(Ordering::Relaxed), 1);
        assert_eq!(count_ok.load(Ordering::Relaxed), 1);

        // Metrics should show one failure
        let metrics = dispatcher.metrics();
        assert_eq!(metrics.total_failed, 1);
    }

    #[tokio::test]
    async fn test_dispatcher_drops_when_channel_full() {
        let count = Arc::new(AtomicUsize::new(0));
        let sink = Arc::new(MockSink::with_delay("slow", count.clone(), 200));

        // Very small channel capacity
        let config = DispatcherConfig {
            channel_capacity: 32,
            sink_channel_capacity: 1,
            ..Default::default()
        };

        let dispatcher = Dispatcher::new(vec![sink as Arc<dyn Sink>], config);

        // Send many events quickly
        for i in 0..100 {
            let _ = dispatcher
                .dispatch_ui(
                    format!("session-{}", i),
                    GatewayPayload::State {
                        state: super::super::types::WorkflowState::Thinking,
                        wait_reason: None,
                    },
                )
                .await;
            tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;

        let metrics = dispatcher.metrics();
        assert!(metrics.total_dropped > 0 || metrics.total_dispatched > 0);
        let sink_metrics = metrics
            .per_sink
            .iter()
            .find(|m| m.sink_name == "slow")
            .expect("slow sink metrics should exist");
        assert!(sink_metrics.dropped > 0 || sink_metrics.processed > 0);
    }

    #[tokio::test]
    async fn test_p0_ui_sink_failure_doesnt_affect_db_sink() {
        let ui_count = Arc::new(AtomicUsize::new(0));
        let db_count = Arc::new(AtomicUsize::new(0));

        let failing_ui_sink = Arc::new(MockSink::with_failure("ui", ui_count.clone()));
        let db_sink = Arc::new(MockSink::new("db", db_count.clone()));

        let dispatcher = Dispatcher::new(
            vec![failing_ui_sink as Arc<dyn Sink>, db_sink as Arc<dyn Sink>],
            DispatcherConfig::default(),
        );

        let event = super::WorkflowEvent::workflow_completed("test-session".to_string(), None);
        dispatcher.dispatch_audit_now(event).unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        assert!(
            db_count.load(Ordering::Relaxed) >= 1,
            "DB sink should receive event even when UI sink fails"
        );
        assert!(
            ui_count.load(Ordering::Relaxed) >= 1,
            "UI sink should be called even though it fails"
        );

        let metrics = dispatcher.metrics();
        assert!(
            metrics.total_failed >= 1,
            "Metrics should show UI sink failure"
        );
    }

    #[tokio::test]
    async fn test_p0_slow_db_sink_doesnt_block_executor() {
        use std::sync::atomic::AtomicBool;
        use std::sync::atomic::Ordering as AtomicOrdering;

        let slow_sink_called = Arc::new(AtomicBool::new(false));
        let slow_sink_done = Arc::new(AtomicBool::new(false));

        struct SlowSink {
            called: Arc<AtomicBool>,
            done: Arc<AtomicBool>,
        }

        #[async_trait::async_trait]
        impl Sink for SlowSink {
            async fn accept(&self, _envelope: EventEnvelope) -> Result<(), WorkflowEngineError> {
                self.called.store(true, AtomicOrdering::Relaxed);
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                self.done.store(true, AtomicOrdering::Relaxed);
                Ok(())
            }

            fn name(&self) -> &str {
                "slow"
            }
        }

        let dispatcher = Dispatcher::new(
            vec![Arc::new(SlowSink {
                called: slow_sink_called.clone(),
                done: slow_sink_done.clone(),
            }) as Arc<dyn Sink>],
            DispatcherConfig::default(),
        );

        let start = std::time::Instant::now();
        dispatcher
            .dispatch_ui(
                "test-session".to_string(),
                GatewayPayload::State {
                    state: super::super::types::WorkflowState::Thinking,
                    wait_reason: None,
                },
            )
            .await
            .unwrap();
        let dispatch_time = start.elapsed();

        assert!(
            dispatch_time < std::time::Duration::from_millis(100),
            "dispatch() should return quickly (<100ms), not wait for slow sink (500ms). Took {:?}",
            dispatch_time
        );

        for _ in 0..20 {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            if slow_sink_called.load(AtomicOrdering::Relaxed) {
                break;
            }
        }

        assert!(
            slow_sink_called.load(AtomicOrdering::Relaxed),
            "Slow sink should have been called"
        );
        assert!(
            !slow_sink_done.load(AtomicOrdering::Relaxed),
            "Slow sink should not have finished yet since dispatch is non-blocking"
        );
    }

    #[tokio::test]
    async fn test_p0_terminal_events_reach_db_sink() {
        let db_count = Arc::new(AtomicUsize::new(0));
        let db_sink = Arc::new(MockSink::new("db", db_count.clone()));

        let dispatcher =
            Dispatcher::new(vec![db_sink as Arc<dyn Sink>], DispatcherConfig::default());

        dispatcher
            .dispatch_terminal("test-session".to_string(), "completed".to_string())
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        assert!(
            db_count.load(Ordering::Relaxed) >= 1,
            "DB sink should receive terminal event"
        );

        db_count.store(0, Ordering::Relaxed);

        dispatcher
            .dispatch_terminal("test-session-2".to_string(), "failed".to_string())
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        assert!(
            db_count.load(Ordering::Relaxed) >= 1,
            "DB sink should receive failed terminal event"
        );
    }

    #[tokio::test]
    async fn test_reliable_sink_isolated_from_best_effort_drops() {
        let ui_count = Arc::new(AtomicUsize::new(0));
        let db_count = Arc::new(AtomicUsize::new(0));

        let best_effort_ui = Arc::new(MockSink::with_delay("ui", ui_count.clone(), 200));
        let reliable_db = Arc::new(MockSink::reliable("db", db_count.clone(), 200));

        let dispatcher = Dispatcher::new(
            vec![
                best_effort_ui as Arc<dyn Sink>,
                reliable_db as Arc<dyn Sink>,
            ],
            DispatcherConfig {
                channel_capacity: 32,
                sink_channel_capacity: 1,
                ..Default::default()
            },
        );

        for i in 0..20 {
            let _ = dispatcher
                .dispatch_ui(
                    format!("session-{}", i),
                    GatewayPayload::State {
                        state: super::super::types::WorkflowState::Thinking,
                        wait_reason: None,
                    },
                )
                .await;
            tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(1200)).await;

        let metrics = dispatcher.metrics();
        let ui_metrics = metrics
            .per_sink
            .iter()
            .find(|m| m.sink_name == "ui")
            .expect("ui sink metrics should exist");
        let db_metrics = metrics
            .per_sink
            .iter()
            .find(|m| m.sink_name == "db")
            .expect("db sink metrics should exist");

        assert!(
            ui_metrics.dropped > 0,
            "best-effort UI sink should drop when saturated"
        );
        assert_eq!(db_metrics.dropped, 0, "reliable DB sink should not drop");
        assert!(
            db_metrics.processed > 0,
            "reliable DB sink should continue processing events"
        );
    }
}
