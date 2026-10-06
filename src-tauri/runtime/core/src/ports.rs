//! Transport-neutral ports owned by the runtime core.
//!
//! Every port is expressed in desktop-free terms only: opaque `Bytes` payloads,
//! session ids as `&str` labels, and an associated error type per port. Nothing
//! in this module names a Tauri, Wry or GTK item, a desktop application type, or
//! a workflow/session type from the desktop crate, so the routing seams can be
//! reviewed before any owner (database, workflow executor, control plane) is
//! moved behind them.
//!
//! The ports deliberately describe *seams*, not policy:
//! - [`RuntimeStore`] is the durability seam. Snapshot and event bodies stay
//!   opaque bytes, so a store adapter never needs workflow types from the
//!   desktop crate.
//! - [`RuntimeEventTransport`] is the outbound seam. One process has exactly one
//!   such route; a payload is encoded before it reaches the port.
//! - [`RuntimeInputPort`] plus [`RuntimeInputSink`] are the inbound seam. The
//!   sink is a local abstraction rather than a concrete channel type, so the
//!   registry stays neutral about which async runtime delivers input.

use std::error::Error;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;

/// One durable event read back from [`RuntimeStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    /// Opaque durable cursor. It is produced by [`RuntimeStore::append_event`]
    /// and handed back to [`RuntimeStore::read_events`].
    pub id: String,
    /// Opaque payload bytes. The runtime core never interprets them.
    pub payload: Bytes,
}

/// Durable workflow state seam.
///
/// The port is byte-oriented on purpose: snapshot and event encoding belongs to
/// the runtime owner, and an adapter (SQLite, in-memory, remote) must not need
/// workflow types from the desktop crate. `Error` stays associated so each
/// adapter keeps its native failure type; the boundary attributes and preserves
/// that failure instead of flattening it.
#[async_trait]
pub trait RuntimeStore: Send + Sync + 'static {
    /// Native failure of the backing store.
    type Error: Error + Send + Sync + 'static;

    /// Latest snapshot written for `session_id`, or `None` when the session has
    /// never been snapshotted.
    async fn load_snapshot(&self, session_id: &str) -> Result<Option<Bytes>, Self::Error>;

    /// Replaces the snapshot stored for `session_id`.
    async fn save_snapshot(&self, session_id: &str, snapshot: Bytes) -> Result<(), Self::Error>;

    /// Appends one durable event and returns its stable id.
    ///
    /// Ids must be ordered so [`RuntimeStore::read_events`] can resume strictly
    /// after the last id a client observed.
    async fn append_event(&self, session_id: &str, event: Bytes) -> Result<String, Self::Error>;

    /// Reads at most `limit` durable events for `session_id`, strictly after
    /// `after`. `after = None` reads from the beginning.
    async fn read_events(
        &self,
        session_id: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredEvent>, Self::Error>;
}

/// Outbound event seam: the single output route of one runtime process.
///
/// Implementations own their own per-session state, so a payload is delivered
/// through exactly one route and a session teardown drops exactly that state.
#[async_trait]
pub trait RuntimeEventTransport: Send + Sync + 'static {
    /// Native failure of this transport.
    type Error: Error + Send + Sync + 'static;

    /// Delivers one already-encoded payload to this process's consumers.
    async fn send(&self, session_id: &str, payload: Bytes) -> Result<(), Self::Error>;

    /// Drops this transport's per-session state, reporting whether anything was
    /// removed.
    async fn remove_session(&self, session_id: &str) -> Result<bool, Self::Error>;
}

/// One session's inbound sink.
///
/// The registry never depends on a concrete channel type: a real adapter wraps
/// whatever the runtime's executor consumes (for example an async mpsc sender),
/// and a test wraps a recorder. A delivery failure means the sink is closed and
/// the caller must treat the session as unreachable.
#[async_trait]
pub trait RuntimeInputSink: Send + Sync + 'static {
    /// Delivers one opaque input payload.
    async fn deliver(&self, input: Bytes) -> Result<(), Box<dyn Error + Send + Sync + 'static>>;
}

/// Inbound seam: the single registry of live session input sinks.
#[async_trait]
pub trait RuntimeInputPort: Send + Sync + 'static {
    /// Native failure of this registry.
    type Error: Error + Send + Sync + 'static;

    /// Registers the sink for `session_id`, reporting whether an existing sink
    /// was replaced.
    async fn register(
        &self,
        session_id: &str,
        sink: Arc<dyn RuntimeInputSink>,
    ) -> Result<bool, Self::Error>;

    /// Removes the sink for `session_id`, reporting whether one existed.
    async fn unregister(&self, session_id: &str) -> Result<bool, Self::Error>;

    /// Delivers `input` to the sink registered for `session_id`.
    async fn inject(&self, session_id: &str, input: Bytes) -> Result<(), Self::Error>;
}
