//! The unique workflow runtime hub.
//!
//! `WorkflowRuntimeHub` is the only [`Gateway`] implementation used by the
//! workflow runtime in production. It owns:
//!
//! - the single registry of live session input senders (one input route per
//!   session, shared by Tauri commands, sub-agent factories and later the HTTP
//!   control plane);
//! - the live SSE event source: every [`GatewayPayload`] published through
//!   [`Gateway::send`] is fanned out as a versioned [`StreamEnvelope`] to a
//!   bounded per-session ring plus a non-blocking `tokio::broadcast` channel.
//!
//! The hub is not a lifecycle registry: session liveness and status remain
//! owned by `WorkflowManager`. Slow SSE subscribers never back-pressure the
//! executor; a lagging or stale subscriber is told to reset via
//! [`SubscribeError::ResetRequired`].
//!
//! Stream cursors (`${server_instance_id}:${sequence}`) are opaque,
//! instance-local replay handles. They are distinct from durable DB event IDs
//! and must never be mixed with them.

use crate::workflow::react::client::tauri::gateway::TauriGateway;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::gateway::Gateway;
use crate::workflow::react::types::GatewayPayload;

use async_trait::async_trait;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex};

/// Schema version of the live stream envelope.
pub const STREAM_SCHEMA_VERSION: u32 = 1;

/// Maximum number of envelopes retained per session for in-window replay.
const RING_CAPACITY: usize = 1024;

/// Capacity of the non-blocking broadcast channel per session.
const BROADCAST_CAPACITY: usize = 256;

/// Versioned envelope published to live stream subscribers.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StreamEnvelope {
    pub schema_version: u32,
    pub server_instance_id: String,
    pub sequence: u64,
    pub session_id: String,
    pub payload: GatewayPayload,
}

impl StreamEnvelope {
    /// Opaque, instance-local replay cursor for this envelope.
    pub fn cursor(&self) -> String {
        format!("{}:{}", self.server_instance_id, self.sequence)
    }
}

/// Error returned when a stream subscription cannot be safely continued.
#[derive(Debug, Clone)]
pub enum SubscribeError {
    /// The cursor is malformed, unknown, from another server instance, or
    /// points outside the retained replay window. The client must re-fetch a
    /// snapshot plus durable events and re-subscribe without a cursor.
    ResetRequired { reason: &'static str },
}

/// A live stream subscription: replayed envelopes plus the live receiver.
#[derive(Debug)]
pub struct SessionSubscription {
    /// Envelopes retained in the ring after `after_cursor`, in sequence order.
    pub replay: Vec<StreamEnvelope>,
    /// Non-blocking receiver for envelopes published after subscription.
    pub rx: broadcast::Receiver<StreamEnvelope>,
}

struct SessionStream {
    ring: VecDeque<StreamEnvelope>,
    tx: broadcast::Sender<StreamEnvelope>,
}

/// Per-session bounded ring + broadcast fan-out of stream envelopes.
///
/// Publishing never blocks and never back-pressures the executor: the ring
/// drops the oldest entries when full and `broadcast::Sender::send` is
/// non-blocking. Lagging subscribers observe `RecvError::Lagged` and must
/// reset.
pub struct SessionEventBroker {
    server_instance_id: String,
    next_sequence: AtomicU64,
    streams: Mutex<HashMap<String, SessionStream>>,
}

impl SessionEventBroker {
    pub fn new(server_instance_id: String) -> Self {
        Self {
            server_instance_id,
            next_sequence: AtomicU64::new(0),
            streams: Mutex::new(HashMap::new()),
        }
    }

    pub fn server_instance_id(&self) -> &str {
        &self.server_instance_id
    }

    /// Publishes an envelope for a session, assigning the next monotonic
    /// sequence. Returns the published envelope.
    pub async fn publish(&self, session_id: &str, payload: GatewayPayload) -> StreamEnvelope {
        let mut streams = self.streams.lock().await;
        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed);
        let envelope = StreamEnvelope {
            schema_version: STREAM_SCHEMA_VERSION,
            server_instance_id: self.server_instance_id.clone(),
            sequence,
            session_id: session_id.to_string(),
            payload,
        };

        let stream = streams.entry(session_id.to_string()).or_insert_with(|| {
            let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
            SessionStream {
                ring: VecDeque::with_capacity(RING_CAPACITY),
                tx,
            }
        });

        if stream.ring.len() >= RING_CAPACITY {
            stream.ring.pop_front();
        }
        stream.ring.push_back(envelope.clone());
        // No live subscribers is normal; a lagging subscriber is handled at
        // receive time via `RecvError::Lagged`.
        let _ = stream.tx.send(envelope.clone());
        envelope
    }

    /// Removes a session's stream. Live subscribers observe channel closure.
    /// Returns `true` when a stream was present.
    pub async fn unregister(&self, session_id: &str) -> bool {
        self.streams.lock().await.remove(session_id).is_some()
    }

    /// Subscribes to a session's live stream, optionally replaying envelopes
    /// after an opaque cursor from the current server instance.
    pub async fn subscribe(
        &self,
        session_id: &str,
        after_cursor: Option<&str>,
    ) -> Result<SessionSubscription, SubscribeError> {
        let mut streams = self.streams.lock().await;
        let after_sequence = match after_cursor {
            None => None,
            Some(cursor) => Some(self.parse_cursor(cursor)?),
        };

        let stream = streams.entry(session_id.to_string()).or_insert_with(|| {
            let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
            SessionStream {
                ring: VecDeque::with_capacity(RING_CAPACITY),
                tx,
            }
        });

        let replay = match after_sequence {
            None => Vec::new(),
            Some(after) => {
                if stream.ring.is_empty() {
                    // The cursor is valid for this instance but the session
                    // stream has no retained history (e.g. re-created after
                    // cleanup): the window cannot be satisfied.
                    return Err(SubscribeError::ResetRequired {
                        reason: "replay_window_unavailable",
                    });
                }
                let oldest = stream.ring.front().expect("ring checked non-empty");
                if after + 1 < oldest.sequence {
                    return Err(SubscribeError::ResetRequired {
                        reason: "replay_window_exceeded",
                    });
                }
                stream
                    .ring
                    .iter()
                    .filter(|envelope| envelope.sequence > after)
                    .cloned()
                    .collect()
            }
        };

        Ok(SessionSubscription {
            replay,
            rx: stream.tx.subscribe(),
        })
    }

    /// Validates an opaque cursor against the current server instance and
    /// returns its sequence.
    fn parse_cursor(&self, cursor: &str) -> Result<u64, SubscribeError> {
        let (instance, sequence) = match cursor.rsplit_once(':') {
            Some(parts) => parts,
            None => {
                return Err(SubscribeError::ResetRequired {
                    reason: "malformed_cursor",
                })
            }
        };
        if instance != self.server_instance_id {
            return Err(SubscribeError::ResetRequired {
                reason: "instance_mismatch",
            });
        }
        let sequence: u64 = match sequence.parse() {
            Ok(value) => value,
            Err(_) => {
                return Err(SubscribeError::ResetRequired {
                    reason: "malformed_cursor",
                })
            }
        };
        if sequence >= self.next_sequence.load(Ordering::Relaxed) {
            return Err(SubscribeError::ResetRequired {
                reason: "unknown_sequence",
            });
        }
        Ok(sequence)
    }
}

/// Registry of live session input senders.
///
/// This is the single input route per session: Tauri commands, sub-agent
/// factories and the HTTP control plane all inject through it. It was moved
/// here from the former `TauriGateway` fields; observable behavior is
/// unchanged.
pub(crate) struct SessionInputRegistry {
    senders: Mutex<HashMap<String, mpsc::Sender<String>>>,
}

impl SessionInputRegistry {
    pub(crate) fn new() -> Self {
        Self {
            senders: Mutex::new(HashMap::new()),
        }
    }

    /// Registers (or replaces) the input sender for a session.
    /// Returns `true` when an existing sender was replaced.
    pub(crate) async fn register(&self, session_id: String, tx: mpsc::Sender<String>) -> bool {
        let mut senders = self.senders.lock().await;
        senders.insert(session_id, tx).is_some()
    }

    /// Removes the input sender for a session.
    /// Returns `true` when a sender was present.
    pub(crate) async fn unregister(&self, session_id: &str) -> bool {
        self.senders.lock().await.remove(session_id).is_some()
    }

    /// Injects a raw input string into the live session's input channel.
    pub(crate) async fn inject(
        &self,
        session_id: &str,
        input: String,
    ) -> Result<(), WorkflowEngineError> {
        let senders = self.senders.lock().await;
        if let Some(tx) = senders.get(session_id) {
            tx.send(input)
                .await
                .map_err(|_| WorkflowEngineError::GatewayInputChannelClosed)?;
            Ok(())
        } else {
            Err(WorkflowEngineError::GatewayInputChannelMissing)
        }
    }
}

/// The unique workflow runtime hub: the only production [`Gateway`].
pub struct WorkflowRuntimeHub {
    /// Tauri output transport (event emission with chunk batching).
    tauri: Arc<TauriGateway>,
    /// Single owner of live session input senders.
    input_registry: SessionInputRegistry,
    /// Live SSE event source.
    broker: Arc<SessionEventBroker>,
}

impl WorkflowRuntimeHub {
    pub fn new(tauri: Arc<TauriGateway>, server_instance_id: String) -> Self {
        Self {
            tauri,
            input_registry: SessionInputRegistry::new(),
            broker: Arc::new(SessionEventBroker::new(server_instance_id)),
        }
    }

    /// Live SSE event source shared by all transport adapters.
    pub fn broker(&self) -> Arc<SessionEventBroker> {
        self.broker.clone()
    }

    pub async fn register_session_tx_with_source(
        &self,
        session_id: String,
        tx: mpsc::Sender<String>,
        source: &str,
    ) {
        let replaced = self.input_registry.register(session_id.clone(), tx).await;
        if replaced {
            log::info!(
                "[Workflow][session={}][phase=gateway][event=signal_channel_replaced] Replaced existing gateway input sender, source={}",
                session_id,
                source
            );
        } else {
            log::info!(
                "[Workflow][session={}][phase=gateway][event=signal_channel_registered] Registered gateway input sender, source={}",
                session_id,
                source
            );
        }
    }

    pub async fn unregister_session_with_source(&self, session_id: &str, source: &str) {
        let input_removed = self.input_registry.unregister(session_id).await;
        let event_removed = self.tauri.remove_event_channel(session_id).await;
        let stream_removed = self.broker.unregister(session_id).await;
        if input_removed || event_removed || stream_removed {
            log::info!(
                "[Workflow][session={}][phase=gateway][event=session_unregistered] Removed gateway channels, input_removed={}, event_removed={}, stream_removed={}, source={}",
                session_id,
                input_removed,
                event_removed,
                stream_removed,
                source
            );
        } else {
            log::debug!(
                "[Workflow][session={}][phase=gateway][event=session_unregister_miss] Gateway channels already missing, source={}",
                session_id,
                source
            );
        }
    }
}

#[async_trait]
impl Gateway for WorkflowRuntimeHub {
    async fn send(
        &self,
        session_id: &str,
        payload: GatewayPayload,
    ) -> Result<(), WorkflowEngineError> {
        // Keep the original Tauri delivery/batching first, then publish the
        // versioned envelope to the live SSE source. Both consumers observe
        // the same payload from a single send call.
        self.tauri.send(session_id, payload.clone()).await?;
        self.broker.publish(session_id, payload).await;
        Ok(())
    }

    async fn register_session_input(
        &self,
        session_id: String,
        tx: mpsc::Sender<String>,
    ) -> Result<(), WorkflowEngineError> {
        self.register_session_tx_with_source(session_id, tx, "sub_agent_factory")
            .await;
        Ok(())
    }

    async fn unregister_session_input(&self, session_id: &str) {
        self.unregister_session_with_source(session_id, "sub_agent_cleanup")
            .await;
    }

    async fn inject_input(
        &self,
        session_id: &str,
        input: String,
    ) -> Result<(), WorkflowEngineError> {
        let signal_type = serde_json::from_str::<serde_json::Value>(&input)
            .ok()
            .and_then(|v| v["type"].as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "raw".to_string());

        log::debug!(
            "[Workflow][session={}][phase=gateway] Injecting signal, type={}",
            session_id,
            signal_type
        );

        match self.input_registry.inject(session_id, input).await {
            Ok(()) => {
                log::debug!(
                    "[Workflow][session={}][phase=gateway] Signal injected successfully, type={}",
                    session_id,
                    signal_type
                );
                Ok(())
            }
            Err(WorkflowEngineError::GatewayInputChannelMissing) => {
                log::info!(
                    "[Workflow][session={}][phase=gateway] No input channel found for session",
                    session_id
                );
                Err(WorkflowEngineError::GatewayInputChannelMissing)
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(content: &str) -> GatewayPayload {
        GatewayPayload::Chunk {
            content: content.to_string(),
        }
    }

    #[tokio::test]
    async fn broker_publishes_envelopes_with_monotonic_sequence_and_cursor() {
        let broker = SessionEventBroker::new("instance-a".to_string());

        let first = broker.publish("s1", chunk("one")).await;
        let second = broker.publish("s1", chunk("two")).await;

        assert_eq!(first.sequence, 0);
        assert_eq!(second.sequence, 1);
        assert_eq!(first.cursor(), "instance-a:0");
        assert_eq!(second.cursor(), "instance-a:1");
        assert_eq!(first.session_id, "s1");
        assert_eq!(first.schema_version, STREAM_SCHEMA_VERSION);
    }

    #[tokio::test]
    async fn broker_replays_ring_after_cursor_and_delivers_live_events() {
        let broker = Arc::new(SessionEventBroker::new("instance-a".to_string()));

        broker.publish("s1", chunk("one")).await;
        broker.publish("s1", chunk("two")).await;

        let mut subscription = broker.subscribe("s1", Some("instance-a:0")).await.unwrap();
        assert_eq!(subscription.replay.len(), 1);
        assert_eq!(subscription.replay[0].sequence, 1);

        broker.publish("s1", chunk("three")).await;
        let live = subscription.rx.recv().await.unwrap();
        assert_eq!(live.sequence, 2);
        assert!(matches!(&live.payload, GatewayPayload::Chunk { content } if content == "three"));
    }

    #[tokio::test]
    async fn broker_subscribe_without_cursor_skips_replay() {
        let broker = SessionEventBroker::new("instance-a".to_string());
        broker.publish("s1", chunk("one")).await;

        let subscription = broker.subscribe("s1", None).await.unwrap();
        assert!(subscription.replay.is_empty());
    }

    #[tokio::test]
    async fn broker_rejects_foreign_instance_and_malformed_cursors() {
        let broker = SessionEventBroker::new("instance-a".to_string());
        broker.publish("s1", chunk("one")).await;

        for cursor in [
            "instance-b:0",
            "not-a-cursor",
            "instance-a:zz",
            "instance-a:99",
        ] {
            let err = broker
                .subscribe("s1", Some(cursor))
                .await
                .expect_err("cursor must be rejected");
            assert!(
                matches!(err, SubscribeError::ResetRequired { .. }),
                "cursor: {}",
                cursor
            );
        }
    }

    #[tokio::test]
    async fn broker_requires_reset_when_replay_window_is_exceeded() {
        let broker = SessionEventBroker::new("instance-a".to_string());
        for i in 0..(RING_CAPACITY + 10) {
            broker.publish("s1", chunk(&i.to_string())).await;
        }

        // A cursor older than the retained window cannot be satisfied.
        let err = broker
            .subscribe("s1", Some("instance-a:0"))
            .await
            .expect_err("stale cursor must reset");
        assert!(matches!(err, SubscribeError::ResetRequired { .. }));

        // A cursor inside the window still replays.
        let kept = broker.subscribe("s1", Some("instance-a:9")).await.unwrap();
        assert_eq!(kept.replay.len(), RING_CAPACITY + 10 - 10);
        assert_eq!(kept.replay[0].sequence, 10);
    }

    #[tokio::test]
    async fn broker_requires_reset_when_stream_history_is_gone() {
        let broker = SessionEventBroker::new("instance-a".to_string());
        broker.publish("s1", chunk("one")).await;
        assert!(broker.unregister("s1").await);

        let err = broker
            .subscribe("s1", Some("instance-a:0"))
            .await
            .expect_err("history must be gone after unregister");
        assert!(matches!(err, SubscribeError::ResetRequired { .. }));
    }

    #[tokio::test]
    async fn broker_lagging_subscriber_observes_lagged_error_instead_of_backpressure() {
        let broker = SessionEventBroker::new("instance-a".to_string());
        let subscription = broker.subscribe("s1", None).await.unwrap();
        let mut rx = subscription.rx;

        // Publish more than the broadcast capacity; publish must never block.
        for i in 0..(BROADCAST_CAPACITY + 32) {
            broker.publish("s1", chunk(&i.to_string())).await;
        }

        let mut saw_lag = false;
        loop {
            match rx.try_recv() {
                Ok(_) => continue,
                Err(broadcast::error::TryRecvError::Lagged(_)) => {
                    saw_lag = true;
                    break;
                }
                Err(broadcast::error::TryRecvError::Empty) => break,
                Err(broadcast::error::TryRecvError::Closed) => panic!("stream must stay open"),
            }
        }
        assert!(saw_lag, "lagging subscriber must observe an explicit lag");
    }

    #[tokio::test]
    async fn broker_keeps_sessions_isolated() {
        let broker = SessionEventBroker::new("instance-a".to_string());
        broker.publish("s1", chunk("one")).await;
        broker.publish("s2", chunk("two")).await;

        let s1 = broker.subscribe("s1", None).await.unwrap();
        assert!(s1.replay.is_empty());
        let s2 = broker.subscribe("s2", Some("instance-a:0")).await.unwrap();
        assert_eq!(s2.replay.len(), 1);
        assert_eq!(s2.replay[0].sequence, 1);
    }

    #[tokio::test]
    async fn registry_register_inject_and_unregister_roundtrip() {
        let registry = SessionInputRegistry::new();
        let (tx, mut rx) = mpsc::channel::<String>(4);

        assert!(!registry.register("s1".to_string(), tx).await);
        registry
            .inject("s1", "hello".to_string())
            .await
            .expect("inject should succeed");
        assert_eq!(rx.recv().await.as_deref(), Some("hello"));

        assert!(registry.unregister("s1").await);
        assert!(!registry.unregister("s1").await);
        let err = registry
            .inject("s1", "again".to_string())
            .await
            .expect_err("missing channel must error");
        assert!(matches!(
            err,
            WorkflowEngineError::GatewayInputChannelMissing
        ));
    }

    #[tokio::test]
    async fn registry_register_replaces_existing_sender() {
        let registry = SessionInputRegistry::new();
        let (tx_old, mut rx_old) = mpsc::channel::<String>(4);
        let (tx_new, mut rx_new) = mpsc::channel::<String>(4);

        assert!(!registry.register("s1".to_string(), tx_old).await);
        assert!(registry.register("s1".to_string(), tx_new).await);

        registry
            .inject("s1", "v2".to_string())
            .await
            .expect("inject should succeed");
        assert_eq!(rx_new.recv().await.as_deref(), Some("v2"));
        // The replaced sender no longer receives input.
        assert!(rx_old.try_recv().is_err());
    }

    #[tokio::test]
    async fn registry_inject_on_closed_channel_returns_closed_error() {
        let registry = SessionInputRegistry::new();
        let (tx, rx) = mpsc::channel::<String>(4);
        drop(rx);

        assert!(!registry.register("s1".to_string(), tx).await);
        let err = registry
            .inject("s1", "orphan".to_string())
            .await
            .expect_err("closed channel must error");
        assert!(matches!(
            err,
            WorkflowEngineError::GatewayInputChannelClosed
        ));
    }
}
