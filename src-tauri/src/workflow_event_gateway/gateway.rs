//! Tauri-specific workflow event transport.
//!
//! This module is a mechanical migration of the former `workflow::react::gateway`
//! `TauriGateway` implementation. It preserves the external Tauri contract:
//! the `workflow://event/{session_id}` event name and chunk/reasoning batching
//! and flush behavior.
//!
//! Since the introduction of the shared runtime hub (`super::hub`), this
//! adapter is output-only: session input senders are owned by the hub's
//! `SessionInputRegistry`, not here.
//!
//! The batching state machine is extracted into a testable helper so the
//! external contract can be characterized by tests without an `AppHandle`.
//! Its observable behavior must stay identical.

use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::types::GatewayPayload;

use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration, Instant};

/// Output sink for Tauri webview events.
///
/// Abstracted so the batching transport can be exercised by tests (and later
/// by the shared hub) without a real `AppHandle`.
pub(crate) trait EventSink: Send + Sync + 'static {
    fn emit(&self, event_name: &str, payload: &GatewayPayload);
}

struct TauriEventSink(AppHandle);

impl EventSink for TauriEventSink {
    fn emit(&self, event_name: &str, payload: &GatewayPayload) {
        let _ = self.0.emit(event_name, payload);
    }
}

/// Canonical Tauri event name for a workflow session's live event stream.
pub fn workflow_event_name(session_id: &str) -> String {
    format!("workflow://event/{}", session_id)
}

/// Batching state machine for the Tauri event stream.
///
/// Extracted verbatim from the former inline batching loop so the external
/// chunk/reasoning batching and flush contract can be characterized by tests
/// without an `AppHandle`.
pub(crate) struct EventBatcher {
    chunk_buffer: String,
    reasoning_buffer: String,
    last_emit: Instant,
    throttle_duration: Duration,
    /// Tracking code block state to avoid partial high-load re-renders
    in_code_block: bool,
}

impl EventBatcher {
    pub(crate) fn new() -> Self {
        Self {
            chunk_buffer: String::new(),
            reasoning_buffer: String::new(),
            last_emit: Instant::now(),
            throttle_duration: Duration::from_millis(100),
            in_code_block: false,
        }
    }

    /// Ingest one payload and return the payloads that must be emitted now.
    pub(crate) fn process(&mut self, payload: GatewayPayload) -> Vec<GatewayPayload> {
        match payload {
            GatewayPayload::Chunk { content } => {
                // Detect code block toggles
                // Note: This is a simple count-based detection for stream batching
                let backtick_count = content.matches("```").count();
                if backtick_count % 2 != 0 {
                    self.in_code_block = !self.in_code_block;
                }

                self.chunk_buffer.push_str(&content);

                // If we are in a code block, we can batch more aggressively
                // If not, or if duration reached, emit
                let should_emit = self.chunk_buffer.len() >= MAX_STREAM_BUFFER_BYTES
                    || (!self.in_code_block
                        && (self.last_emit.elapsed() >= self.throttle_duration
                            || self.chunk_buffer.len() > 500));

                if should_emit {
                    let emitted = vec![GatewayPayload::Chunk {
                        content: std::mem::take(&mut self.chunk_buffer),
                    }];
                    self.last_emit = Instant::now();
                    return emitted;
                }
                Vec::new()
            }
            GatewayPayload::ReasoningChunk { content } => {
                self.reasoning_buffer.push_str(&content);
                if self.last_emit.elapsed() >= self.throttle_duration
                    || self.reasoning_buffer.len() >= MAX_STREAM_BUFFER_BYTES
                {
                    let emitted = vec![GatewayPayload::ReasoningChunk {
                        content: std::mem::take(&mut self.reasoning_buffer),
                    }];
                    self.last_emit = Instant::now();
                    return emitted;
                }
                Vec::new()
            }
            GatewayPayload::Notification { .. } => {
                // Notifications should be sent immediately
                vec![payload]
            }
            // Non-chunk messages (State, Confirm, Full Messages) should flush buffers and be sent immediately
            payload => {
                let mut emissions = self.flush();
                emissions.push(payload);
                self.last_emit = Instant::now();
                emissions
            }
        }
    }

    /// Flush any buffered chunk/reasoning content as pending emissions.
    pub(crate) fn flush(&mut self) -> Vec<GatewayPayload> {
        let mut emissions = Vec::new();
        if !self.chunk_buffer.is_empty() {
            emissions.push(GatewayPayload::Chunk {
                content: std::mem::take(&mut self.chunk_buffer),
            });
        }
        if !self.reasoning_buffer.is_empty() {
            emissions.push(GatewayPayload::ReasoningChunk {
                content: std::mem::take(&mut self.reasoning_buffer),
            });
        }
        emissions
    }
}

const MAX_STREAM_BUFFER_BYTES: usize = 64 * 1024;

/// Tauri output transport for workflow events, with throttling and batching.
///
/// Input ownership lives in the shared runtime hub; this adapter only emits
/// events to the Tauri webview.
pub struct TauriGateway {
    sink: Arc<dyn EventSink>,
    /// Map of session_id -> mpsc sender for aggregated events.
    event_senders: Mutex<HashMap<String, mpsc::Sender<GatewayPayload>>>,
}

impl TauriGateway {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            sink: Arc::new(TauriEventSink(app_handle)),
            event_senders: Mutex::new(HashMap::new()),
        }
    }

    /// Test-only constructor with a custom event sink.
    #[cfg(test)]
    pub(crate) fn with_sink(sink: Arc<dyn EventSink>) -> Self {
        Self {
            sink,
            event_senders: Mutex::new(HashMap::new()),
        }
    }

    /// Sends an event to the Tauri webview for a session, batching stream
    /// chunks and flushing on control messages.
    pub async fn send(
        &self,
        session_id: &str,
        payload: GatewayPayload,
    ) -> Result<(), WorkflowEngineError> {
        let tx = self.get_or_create_event_channel(session_id).await;
        tx.send(payload)
            .await
            .map_err(|e| WorkflowEngineError::Gateway(e.to_string()))?;
        Ok(())
    }

    /// Removes the aggregated event channel for a session, ending its batching
    /// task after a final flush. Returns `true` when a channel was present.
    pub async fn remove_event_channel(&self, session_id: &str) -> bool {
        self.event_senders.lock().await.remove(session_id).is_some()
    }

    /// Create or get an aggregated channel for a session
    async fn get_or_create_event_channel(&self, session_id: &str) -> mpsc::Sender<GatewayPayload> {
        let mut senders = self.event_senders.lock().await;
        if let Some(tx) = senders.get(session_id) {
            return tx.clone();
        }

        let (tx, mut rx) = mpsc::channel::<GatewayPayload>(1000);
        let sink = self.sink.clone();
        let event_name = workflow_event_name(session_id);

        tokio::spawn(async move {
            let mut batcher = EventBatcher::new();

            while let Some(payload) = rx.recv().await {
                for emission in batcher.process(payload) {
                    sink.emit(&event_name, &emission);
                }

                // Small sleep to yield
                sleep(Duration::from_millis(5)).await;
            }

            for emission in batcher.flush() {
                sink.emit(&event_name, &emission);
            }
        });

        senders.insert(session_id.to_string(), tx.clone());
        tx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_event_name_formats_canonical_tauri_event() {
        assert_eq!(
            workflow_event_name("session-123"),
            "workflow://event/session-123"
        );
    }

    fn chunk(content: &str) -> GatewayPayload {
        GatewayPayload::Chunk {
            content: content.to_string(),
        }
    }

    fn chunk_content(payload: &GatewayPayload) -> String {
        match payload {
            GatewayPayload::Chunk { content } => content.clone(),
            other => panic!("expected Chunk payload, got {:?}", other),
        }
    }

    #[test]
    fn batcher_buffers_small_chunks_within_throttle_window() {
        let mut batcher = EventBatcher::new();
        assert!(batcher.process(chunk("hello ")).is_empty());
        assert!(batcher.process(chunk("world")).is_empty());
        // Channel close flushes the buffered content as one chunk.
        let flushed = batcher.flush();
        assert_eq!(flushed.len(), 1);
        assert_eq!(chunk_content(&flushed[0]), "hello world");
    }

    #[test]
    fn batcher_emits_large_chunk_immediately_outside_code_block() {
        let mut batcher = EventBatcher::new();
        let large = "x".repeat(501);
        let emissions = batcher.process(chunk(&large));
        assert_eq!(emissions.len(), 1);
        assert_eq!(chunk_content(&emissions[0]), large);
        // Buffer is cleared after emission.
        assert!(batcher.flush().is_empty());
    }

    #[test]
    fn batcher_batches_inside_code_block_until_buffer_limit() {
        let mut batcher = EventBatcher::new();
        // Odd number of ``` toggles code block state on.
        assert!(batcher.process(chunk("```rust\n")).is_empty());
        let large = "x".repeat(501);
        // Inside a code block, size alone does not trigger emission.
        assert!(batcher.process(chunk(&large)).is_empty());
        // Closing the fence toggles code block state off; since the buffer now
        // exceeds the size threshold outside a code block, this same chunk
        // emits everything buffered so far.
        let emissions = batcher.process(chunk("\n```"));
        assert_eq!(emissions.len(), 1);
        assert_eq!(
            chunk_content(&emissions[0]),
            format!("```rust\n{}\n```", large)
        );
        // Subsequent small chunks are buffered again within the throttle window.
        assert!(batcher.process(chunk("y")).is_empty());
        let flushed = batcher.flush();
        assert_eq!(chunk_content(&flushed[0]), "y");
    }

    #[test]
    fn batcher_control_payload_flushes_buffers_before_payload() {
        let mut batcher = EventBatcher::new();
        assert!(batcher.process(chunk("partial")).is_empty());
        assert!(batcher
            .process(GatewayPayload::ReasoningChunk {
                content: "thinking".to_string()
            })
            .is_empty());

        let state = GatewayPayload::State {
            state: crate::workflow::react::types::WorkflowState::Executing,
            wait_reason: None,
        };
        let emissions = batcher.process(state);
        assert_eq!(emissions.len(), 3);
        assert_eq!(chunk_content(&emissions[0]), "partial");
        assert!(matches!(
            &emissions[1],
            GatewayPayload::ReasoningChunk { content } if content == "thinking"
        ));
        assert!(matches!(emissions[2], GatewayPayload::State { .. }));
    }

    #[test]
    fn batcher_notification_is_immediate_and_does_not_flush_buffers() {
        let mut batcher = EventBatcher::new();
        assert!(batcher.process(chunk("partial")).is_empty());

        let notification = GatewayPayload::Notification {
            message: "note".to_string(),
            category: None,
        };
        let emissions = batcher.process(notification);
        assert_eq!(emissions.len(), 1);
        assert!(matches!(emissions[0], GatewayPayload::Notification { .. }));

        // Buffered chunk is untouched and still flushed on close.
        let flushed = batcher.flush();
        assert_eq!(flushed.len(), 1);
        assert_eq!(chunk_content(&flushed[0]), "partial");
    }

    #[tokio::test]
    async fn batcher_emits_chunk_after_throttle_window_elapses() {
        let mut batcher = EventBatcher::new();
        assert!(batcher.process(chunk("first")).is_empty());
        tokio::time::sleep(Duration::from_millis(120)).await;
        let emissions = batcher.process(chunk("second"));
        assert_eq!(emissions.len(), 1);
        assert_eq!(chunk_content(&emissions[0]), "firstsecond");
    }
}
