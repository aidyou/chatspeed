//! Transport-neutral runtime core (U-3 ports-first slice).
//!
//! This crate defines the desktop-free boundary of the ChatSpeed runtime: the
//! three ports a runtime needs to serve a client (durability, outbound events,
//! inbound input) and [`RuntimeFacade`], the single place where those ports are
//! composed into one canonical route.
//!
//! ## What this crate is
//!
//! - A **ports-only** boundary. It contains no executor, no registry
//!   implementation, no HTTP surface and no persistence adapter; it defines the
//!   seams those owners will be moved behind.
//! - **Desktop-free by policy**: the production dependency graph must not
//!   contain `tauri`, `wry`, `gtk` or the `chatspeed` desktop crate, and none of
//!   its own modules name those items.
//! - **Schema-reusing**: wire types (`ErrorEnvelope`, `StreamEnvelope`) come
//!   from `chatspeed-contracts`. This crate does not redeclare them; it only
//!   adds the lifecycle vocabulary the runtime needs and converts its own
//!   failures into the shared envelope.
//!
//! ## What this crate is not
//!
//! Nothing here has migrated an owner. `MainStore`, the workflow runtime hub,
//! the application service and the `/control/v1` server still live in the
//! desktop crate and are still created by Tauri `setup()`. This slice therefore
//! makes **no** claim about AC-6 (live plus durable workflow events) or AC-7
//! (machine-stable CLI output): both require the real owners to move behind
//! these ports, which is out of scope here.

mod ports;

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use bytes::Bytes;
use chatspeed_contracts::{ErrorEnvelope, StreamEnvelope};
use serde::{Deserialize, Serialize};

pub use ports::{
    RuntimeEventTransport, RuntimeInputPort, RuntimeInputSink, RuntimeStore, StoredEvent,
};

/// The live stream envelope version is owned by `chatspeed-contracts`.
pub use chatspeed_contracts::STREAM_SCHEMA_VERSION;

// ---------------------------------------------------------------------------
// Lifecycle vocabulary
// ---------------------------------------------------------------------------

/// Lifecycle of one runtime process, with stable snake_case wire names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeLifecycle {
    /// Binding, locking and recovering; not serving work yet.
    Starting,
    /// Fully serving clients.
    Ready,
    /// Serving with a reduced capability set.
    Degraded,
    /// Draining: no new work is accepted.
    ShuttingDown,
    /// Exited or unusable.
    Stopped,
}

impl RuntimeLifecycle {
    /// Whether this state may accept new work.
    ///
    /// Only [`RuntimeLifecycle::Ready`] serves work; every other state fails
    /// closed instead of half-serving a client, which is the same posture the
    /// runtime takes for its directory lock and discovery document.
    pub fn accepts_work(self) -> bool {
        matches!(self, Self::Ready)
    }

    /// Stable snake_case name, identical to the serialized wire form.
    pub fn as_code(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::ShuttingDown => "shutting_down",
            Self::Stopped => "stopped",
        }
    }
}

impl fmt::Display for RuntimeLifecycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_code())
    }
}

/// Lifecycle phase of one workflow session as seen by clients.
///
/// This is the minimal stable vocabulary published on the canonical outbound
/// route; it carries no workflow-internal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLifecycle {
    /// Session accepted, not running yet.
    Accepted,
    /// Executing.
    Running,
    /// Blocked on further client input.
    AwaitingInput,
    /// Finished successfully.
    Completed,
    /// Finished with an error.
    Failed,
    /// Cancelled by a client or by shutdown.
    Cancelled,
}

/// Payload of a session lifecycle event.
///
/// It is deliberately one field: the envelope already carries the session id,
/// the instance id and the sequence, so the payload only states the transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SessionLifecycleEvent {
    /// The phase the session moved into.
    pub phase: SessionLifecycle,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Which port produced a failure, with a stable snake_case name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimePort {
    /// [`RuntimeStore`].
    Store,
    /// [`RuntimeEventTransport`].
    Transport,
    /// [`RuntimeInputPort`].
    Input,
}

impl RuntimePort {
    /// Stable snake_case name, identical to the serialized wire form.
    pub fn as_code(self) -> &'static str {
        match self {
            Self::Store => "store",
            Self::Transport => "transport",
            Self::Input => "input",
        }
    }
}

impl fmt::Display for RuntimePort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_code())
    }
}

/// Structured failure of the runtime boundary.
///
/// Every failure names the port or precondition that produced it, keeps the
/// native cause in the error chain, and maps to one stable snake_case code, so
/// clients never have to parse a message to classify it.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeCoreError {
    /// A port returned its own native error.
    #[error("{port} port failed during {operation}: {message}")]
    Port {
        /// Port that failed.
        port: RuntimePort,
        /// Boundary operation that was running.
        operation: &'static str,
        /// Rendered native failure, kept for the envelope message.
        message: String,
        /// Native cause, preserved for the chain.
        #[source]
        source: Option<Box<dyn Error + Send + Sync + 'static>>,
    },
    /// A payload could not be encoded for the wire.
    #[error("{operation} could not encode its payload: {message}")]
    Serialization {
        /// Boundary operation that was running.
        operation: &'static str,
        /// Encoder failure text.
        message: String,
    },
    /// The lifecycle state refuses to serve the requested work.
    #[error("runtime cannot {operation} while the lifecycle is {state}")]
    NotReady {
        /// Boundary operation that was refused.
        operation: &'static str,
        /// Lifecycle state that refused it.
        state: RuntimeLifecycle,
    },
}

impl RuntimeCoreError {
    /// Attributes a [`RuntimeStore`] failure to the durability port.
    pub fn store(operation: &'static str, error: impl Error + Send + Sync + 'static) -> Self {
        Self::port(RuntimePort::Store, operation, error)
    }

    /// Attributes an outbound failure to the event transport port.
    pub fn transport(operation: &'static str, error: impl Error + Send + Sync + 'static) -> Self {
        Self::port(RuntimePort::Transport, operation, error)
    }

    /// Attributes an inbound failure to the input port.
    pub fn input(operation: &'static str, error: impl Error + Send + Sync + 'static) -> Self {
        Self::port(RuntimePort::Input, operation, error)
    }

    /// Records a payload encoding failure.
    pub fn serialization(operation: &'static str, error: serde_json::Error) -> Self {
        Self::Serialization {
            operation,
            message: error.to_string(),
        }
    }

    /// Records a refusal from the lifecycle gate.
    pub fn not_ready(operation: &'static str, state: RuntimeLifecycle) -> Self {
        Self::NotReady { operation, state }
    }

    fn port(
        port: RuntimePort,
        operation: &'static str,
        error: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self::Port {
            port,
            operation,
            message: error.to_string(),
            source: Some(Box::new(error)),
        }
    }

    /// Stable snake_case code classifying this failure.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Port { port, .. } => match port {
                RuntimePort::Store => "store_error",
                RuntimePort::Transport => "transport_error",
                RuntimePort::Input => "input_error",
            },
            Self::Serialization { .. } => "serialization_error",
            Self::NotReady { .. } => "not_ready",
        }
    }

    /// Port this failure came from, when it came from a port.
    pub fn port_kind(&self) -> Option<RuntimePort> {
        match self {
            Self::Port { port, .. } => Some(*port),
            _ => None,
        }
    }

    /// Converts to the shared canonical error envelope.
    ///
    /// The envelope type comes from `chatspeed-contracts`; this crate never
    /// defines a second error schema.
    pub fn to_envelope(&self) -> ErrorEnvelope {
        ErrorEnvelope::new(self.code(), self.to_string())
    }
}

// ---------------------------------------------------------------------------
// Boundary
// ---------------------------------------------------------------------------

/// Result of releasing one session's runtime route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnregisterOutcome {
    /// Whether an inbound sink was registered and removed.
    pub input_removed: bool,
    /// Whether the transport held per-session state that was removed.
    pub transport_removed: bool,
}

/// The single composition point of the three runtime ports.
///
/// The facade exists so the runtime has exactly one route in each direction and
/// exactly one teardown path, instead of every owner wiring its own:
///
/// - inbound work (`register_input`, `inject_input`) goes through one
///   [`RuntimeInputPort`];
/// - outbound events (`publish_payload`, `publish_lifecycle`) go through one
///   [`RuntimeEventTransport`];
/// - durability (`save_snapshot`, `load_snapshot`, `append_event`,
///   `read_events`) goes through one [`RuntimeStore`];
/// - `unregister_session` releases the inbound sink and the transport's session
///   state together, so a session can never be left half-registered.
///
/// Serving methods fail closed with [`RuntimeCoreError::NotReady`] unless the
/// lifecycle is [`RuntimeLifecycle::Ready`]. Teardown and reads stay available
/// in every state so a drain or a shutdown can still complete.
pub struct RuntimeFacade<S, T, I>
where
    S: RuntimeStore,
    T: RuntimeEventTransport,
    I: RuntimeInputPort,
{
    store: Arc<S>,
    transport: Arc<T>,
    input: Arc<I>,
    server_instance_id: String,
    lifecycle: RuntimeLifecycle,
}

impl<S, T, I> RuntimeFacade<S, T, I>
where
    S: RuntimeStore,
    T: RuntimeEventTransport,
    I: RuntimeInputPort,
{
    /// Composes the ports into one boundary.
    pub fn new(
        store: Arc<S>,
        transport: Arc<T>,
        input: Arc<I>,
        server_instance_id: impl Into<String>,
        lifecycle: RuntimeLifecycle,
    ) -> Self {
        Self {
            store,
            transport,
            input,
            server_instance_id: server_instance_id.into(),
            lifecycle,
        }
    }

    /// Instance id stamped on outbound envelopes.
    pub fn server_instance_id(&self) -> &str {
        &self.server_instance_id
    }

    /// Lifecycle state this boundary is in.
    pub fn lifecycle(&self) -> RuntimeLifecycle {
        self.lifecycle
    }

    /// Registers, or replaces, the inbound sink of one session.
    pub async fn register_input(
        &self,
        session_id: &str,
        sink: Arc<dyn RuntimeInputSink>,
    ) -> Result<bool, RuntimeCoreError> {
        self.ensure_ready("register_session_input")?;
        self.input
            .register(session_id, sink)
            .await
            .map_err(|error| RuntimeCoreError::input("register_session_input", error))
    }

    /// Delivers one raw input payload to a session's registered sink.
    pub async fn inject_input(
        &self,
        session_id: &str,
        input: Bytes,
    ) -> Result<(), RuntimeCoreError> {
        self.ensure_ready("inject_input")?;
        self.input
            .inject(session_id, input)
            .await
            .map_err(|error| RuntimeCoreError::input("inject_input", error))
    }

    /// Releases both routes of one session.
    ///
    /// This is the only teardown path: the inbound sink and the transport's
    /// per-session state are always released together, and it stays available
    /// outside [`RuntimeLifecycle::Ready`] so a drain can finish.
    pub async fn unregister_session(
        &self,
        session_id: &str,
    ) -> Result<UnregisterOutcome, RuntimeCoreError> {
        let input_removed = self
            .input
            .unregister(session_id)
            .await
            .map_err(|error| RuntimeCoreError::input("unregister_session_input", error))?;
        let transport_removed = self
            .transport
            .remove_session(session_id)
            .await
            .map_err(|error| RuntimeCoreError::transport("remove_session", error))?;
        Ok(UnregisterOutcome {
            input_removed,
            transport_removed,
        })
    }

    /// Publishes one already-encoded payload on the canonical outbound route.
    pub async fn publish_payload(
        &self,
        session_id: &str,
        payload: Bytes,
    ) -> Result<(), RuntimeCoreError> {
        self.ensure_ready("publish_event")?;
        self.transport
            .send(session_id, payload)
            .await
            .map_err(|error| RuntimeCoreError::transport("publish_event", error))
    }

    /// Publishes a session lifecycle transition on the same canonical route,
    /// wrapped in the envelope shared with the control plane.
    pub async fn publish_lifecycle(
        &self,
        session_id: &str,
        sequence: u64,
        phase: SessionLifecycle,
    ) -> Result<(), RuntimeCoreError> {
        self.ensure_ready("publish_lifecycle")?;
        let payload = serde_json::to_value(SessionLifecycleEvent { phase })
            .map_err(|error| RuntimeCoreError::serialization("publish_lifecycle", error))?;
        let envelope = StreamEnvelope {
            schema_version: STREAM_SCHEMA_VERSION,
            server_instance_id: self.server_instance_id.clone(),
            sequence,
            session_id: session_id.to_string(),
            payload,
        };
        let bytes = serde_json::to_vec(&envelope)
            .map_err(|error| RuntimeCoreError::serialization("publish_lifecycle", error))?;
        self.publish_payload(session_id, Bytes::from(bytes)).await
    }

    /// Replaces the stored snapshot of one session.
    pub async fn save_snapshot(
        &self,
        session_id: &str,
        snapshot: Bytes,
    ) -> Result<(), RuntimeCoreError> {
        self.ensure_ready("save_snapshot")?;
        self.store
            .save_snapshot(session_id, snapshot)
            .await
            .map_err(|error| RuntimeCoreError::store("save_snapshot", error))
    }

    /// Reads the stored snapshot of one session.
    ///
    /// Reads stay available outside [`RuntimeLifecycle::Ready`] so a client can
    /// still recover state while the runtime drains.
    pub async fn load_snapshot(&self, session_id: &str) -> Result<Option<Bytes>, RuntimeCoreError> {
        self.store
            .load_snapshot(session_id)
            .await
            .map_err(|error| RuntimeCoreError::store("load_snapshot", error))
    }

    /// Appends one durable event, returning its stable id.
    pub async fn append_event(
        &self,
        session_id: &str,
        event: Bytes,
    ) -> Result<String, RuntimeCoreError> {
        self.ensure_ready("append_event")?;
        self.store
            .append_event(session_id, event)
            .await
            .map_err(|error| RuntimeCoreError::store("append_event", error))
    }

    /// Reads durable events strictly after `after`.
    ///
    /// Reads stay available outside [`RuntimeLifecycle::Ready`] so event replay
    /// is not cut off by a drain.
    pub async fn read_events(
        &self,
        session_id: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredEvent>, RuntimeCoreError> {
        self.store
            .read_events(session_id, after, limit)
            .await
            .map_err(|error| RuntimeCoreError::store("read_events", error))
    }

    fn ensure_ready(&self, operation: &'static str) -> Result<(), RuntimeCoreError> {
        if self.lifecycle.accepts_work() {
            Ok(())
        } else {
            Err(RuntimeCoreError::not_ready(operation, self.lifecycle))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Mutex;

    use async_trait::async_trait;
    use serde_json::json;

    use super::*;

    /// Native failure of the fakes, standing in for a real adapter error.
    #[derive(Debug)]
    struct PortFailure(String);

    impl fmt::Display for PortFailure {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl Error for PortFailure {}

    #[derive(Default)]
    struct FakeStore {
        snapshots: Mutex<BTreeMap<String, Bytes>>,
        events: Mutex<Vec<(String, Bytes)>>,
        offline: bool,
    }

    impl FakeStore {
        fn offline() -> Self {
            Self {
                offline: true,
                ..Self::default()
            }
        }
    }

    #[async_trait]
    impl RuntimeStore for FakeStore {
        type Error = PortFailure;

        async fn load_snapshot(&self, session_id: &str) -> Result<Option<Bytes>, Self::Error> {
            if self.offline {
                return Err(PortFailure("store offline".to_string()));
            }
            Ok(self
                .snapshots
                .lock()
                .expect("snapshot lock")
                .get(session_id)
                .cloned())
        }

        async fn save_snapshot(
            &self,
            session_id: &str,
            snapshot: Bytes,
        ) -> Result<(), Self::Error> {
            if self.offline {
                return Err(PortFailure("store offline".to_string()));
            }
            self.snapshots
                .lock()
                .expect("snapshot lock")
                .insert(session_id.to_string(), snapshot);
            Ok(())
        }

        async fn append_event(
            &self,
            session_id: &str,
            event: Bytes,
        ) -> Result<String, Self::Error> {
            if self.offline {
                return Err(PortFailure("store offline".to_string()));
            }
            let mut events = self.events.lock().expect("event lock");
            let id = format!("ev-{}", events.len() + 1);
            events.push((session_id.to_string(), event));
            Ok(id)
        }

        async fn read_events(
            &self,
            session_id: &str,
            after: Option<&str>,
            limit: usize,
        ) -> Result<Vec<StoredEvent>, Self::Error> {
            if self.offline {
                return Err(PortFailure("store offline".to_string()));
            }
            let events = self.events.lock().expect("event lock");
            let mut read = Vec::new();
            for (index, (session, payload)) in events.iter().enumerate() {
                if session != session_id {
                    continue;
                }
                let id = format!("ev-{}", index + 1);
                if after.is_some_and(|after| id.as_str() <= after) {
                    continue;
                }
                read.push(StoredEvent {
                    id,
                    payload: payload.clone(),
                });
                if read.len() == limit {
                    break;
                }
            }
            Ok(read)
        }
    }

    #[derive(Default)]
    struct FakeTransport {
        attempts: Mutex<Vec<(String, Bytes)>>,
        delivered: Mutex<Vec<(String, Bytes)>>,
        live: Mutex<BTreeSet<String>>,
        removed: Mutex<u32>,
        closed: bool,
    }

    impl FakeTransport {
        fn closed() -> Self {
            Self {
                closed: true,
                ..Self::default()
            }
        }
    }

    #[async_trait]
    impl RuntimeEventTransport for FakeTransport {
        type Error = PortFailure;

        async fn send(&self, session_id: &str, payload: Bytes) -> Result<(), Self::Error> {
            self.attempts
                .lock()
                .expect("attempt lock")
                .push((session_id.to_string(), payload.clone()));
            if self.closed {
                return Err(PortFailure("transport closed".to_string()));
            }
            self.delivered
                .lock()
                .expect("delivery lock")
                .push((session_id.to_string(), payload));
            Ok(())
        }

        async fn remove_session(&self, session_id: &str) -> Result<bool, Self::Error> {
            *self.removed.lock().expect("remove lock") += 1;
            Ok(self
                .live
                .lock()
                .expect("live lock")
                .insert(session_id.to_string()))
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        delivered: Mutex<Vec<Bytes>>,
    }

    impl RecordingSink {
        fn delivered(&self) -> Vec<Bytes> {
            self.delivered.lock().expect("sink lock").clone()
        }
    }

    #[async_trait]
    impl RuntimeInputSink for RecordingSink {
        async fn deliver(
            &self,
            input: Bytes,
        ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
            self.delivered.lock().expect("sink lock").push(input);
            Ok(())
        }
    }

    /// In-memory input registry: the fake the real registry must match.
    #[derive(Default)]
    struct FakeInputPort {
        sinks: Mutex<BTreeMap<String, Arc<dyn RuntimeInputSink>>>,
    }

    #[async_trait]
    impl RuntimeInputPort for FakeInputPort {
        type Error = PortFailure;

        async fn register(
            &self,
            session_id: &str,
            sink: Arc<dyn RuntimeInputSink>,
        ) -> Result<bool, Self::Error> {
            Ok(self
                .sinks
                .lock()
                .expect("registry lock")
                .insert(session_id.to_string(), sink)
                .is_some())
        }

        async fn unregister(&self, session_id: &str) -> Result<bool, Self::Error> {
            Ok(self
                .sinks
                .lock()
                .expect("registry lock")
                .remove(session_id)
                .is_some())
        }

        async fn inject(&self, session_id: &str, input: Bytes) -> Result<(), Self::Error> {
            // Clone the sink out of the registry so no lock is held across the
            // delivery await.
            let sink = self
                .sinks
                .lock()
                .expect("registry lock")
                .get(session_id)
                .cloned()
                .ok_or_else(|| PortFailure(format!("no input channel for {session_id}")))?;
            sink.deliver(input)
                .await
                .map_err(|error| PortFailure(error.to_string()))
        }
    }

    type Facade = RuntimeFacade<FakeStore, FakeTransport, FakeInputPort>;

    struct Harness {
        facade: Facade,
        store: Arc<FakeStore>,
        transport: Arc<FakeTransport>,
    }

    fn harness(lifecycle: RuntimeLifecycle, store: FakeStore, transport: FakeTransport) -> Harness {
        let store = Arc::new(store);
        let transport = Arc::new(transport);
        let input = Arc::new(FakeInputPort::default());
        let facade = RuntimeFacade::new(
            store.clone(),
            transport.clone(),
            input,
            "instance-a",
            lifecycle,
        );
        Harness {
            facade,
            store,
            transport,
        }
    }

    fn ready() -> Harness {
        harness(
            RuntimeLifecycle::Ready,
            FakeStore::default(),
            FakeTransport::default(),
        )
    }

    #[tokio::test]
    async fn input_is_registered_injected_and_unregistered_through_one_route() {
        let harness = ready();
        let sink = Arc::new(RecordingSink::default());

        assert!(
            !harness
                .facade
                .register_input("s1", sink.clone())
                .await
                .expect("first registration"),
            "the first registration replaces nothing"
        );
        assert!(
            harness
                .facade
                .register_input("s1", sink.clone())
                .await
                .expect("second registration"),
            "re-registering the same session reports the replacement"
        );

        let injected = Bytes::from_static(b"{\"type\":\"resume\"}");
        harness
            .facade
            .inject_input("s1", injected.clone())
            .await
            .expect("inject into the registered sink");
        assert_eq!(sink.delivered(), vec![injected]);

        let outcome = harness
            .facade
            .unregister_session("s1")
            .await
            .expect("unregister");
        assert_eq!(
            outcome,
            UnregisterOutcome {
                input_removed: true,
                transport_removed: true,
            }
        );
        assert_eq!(
            *harness.transport.removed.lock().expect("remove lock"),
            1,
            "teardown touches the transport exactly once"
        );

        let error = harness
            .facade
            .inject_input("s1", Bytes::from_static(b"{}"))
            .await
            .expect_err("the sink is gone");
        assert_eq!(error.code(), "input_error");
        assert_eq!(error.port_kind(), Some(RuntimePort::Input));
        assert!(error.to_string().contains("no input channel for s1"));
        assert_eq!(sink.delivered().len(), 1, "no delivery after teardown");
    }

    #[tokio::test]
    async fn outbound_payloads_take_one_route_and_are_unchanged() {
        let harness = ready();

        harness
            .facade
            .publish_payload("s1", Bytes::from_static(b"raw-1"))
            .await
            .expect("publish to s1");
        harness
            .facade
            .publish_payload("s2", Bytes::from_static(b"raw-2"))
            .await
            .expect("publish to s2");

        let delivered = harness.transport.delivered.lock().expect("delivery lock");
        assert_eq!(
            *delivered,
            vec![
                ("s1".to_string(), Bytes::from_static(b"raw-1")),
                ("s2".to_string(), Bytes::from_static(b"raw-2")),
            ],
            "one publish is exactly one transport delivery, byte for byte"
        );
    }

    #[tokio::test]
    async fn transport_failure_propagates_with_its_native_cause() {
        let harness = harness(
            RuntimeLifecycle::Ready,
            FakeStore::default(),
            FakeTransport::closed(),
        );

        let error = harness
            .facade
            .publish_payload("s1", Bytes::from_static(b"{}"))
            .await
            .expect_err("the transport is closed");

        assert_eq!(error.code(), "transport_error");
        assert_eq!(error.port_kind(), Some(RuntimePort::Transport));
        assert!(error.to_string().contains("transport closed"));
        assert!(
            error.source().is_some(),
            "the native transport error stays in the chain"
        );
        assert_eq!(
            harness
                .transport
                .attempts
                .lock()
                .expect("attempt lock")
                .len(),
            1,
            "a failed publish is still exactly one transport call"
        );
        assert!(harness
            .transport
            .delivered
            .lock()
            .expect("delivery lock")
            .is_empty());
    }

    #[tokio::test]
    async fn store_failure_propagates_with_its_native_cause() {
        let harness = harness(
            RuntimeLifecycle::Ready,
            FakeStore::offline(),
            FakeTransport::default(),
        );

        let error = harness
            .facade
            .save_snapshot("s1", Bytes::from_static(b"snap"))
            .await
            .expect_err("the store is offline");

        assert_eq!(error.code(), "store_error");
        assert_eq!(error.port_kind(), Some(RuntimePort::Store));
        assert!(error.to_string().contains("store offline"));
        assert!(error.source().is_some());
        assert!(harness
            .store
            .snapshots
            .lock()
            .expect("snapshot lock")
            .is_empty());
    }

    #[tokio::test]
    async fn store_round_trips_opaque_bytes_through_the_single_route() {
        let harness = ready();

        harness
            .facade
            .save_snapshot("s1", Bytes::from_static(b"snapshot-1"))
            .await
            .expect("save");
        assert_eq!(
            harness
                .facade
                .load_snapshot("s1")
                .await
                .expect("load")
                .as_deref(),
            Some(&b"snapshot-1"[..])
        );

        let id = harness
            .facade
            .append_event("s1", Bytes::from_static(b"event-1"))
            .await
            .expect("append");
        let events = harness
            .facade
            .read_events("s1", None, 10)
            .await
            .expect("read");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, id);
        assert_eq!(events[0].payload.as_ref(), &b"event-1"[..]);
    }

    #[tokio::test]
    async fn non_ready_lifecycles_fail_closed_while_teardown_and_reads_still_work() {
        for state in [
            RuntimeLifecycle::Starting,
            RuntimeLifecycle::Degraded,
            RuntimeLifecycle::ShuttingDown,
            RuntimeLifecycle::Stopped,
        ] {
            let harness = harness(state, FakeStore::default(), FakeTransport::default());
            assert_eq!(harness.facade.lifecycle(), state);
            assert_eq!(harness.facade.server_instance_id(), "instance-a");

            let sink = Arc::new(RecordingSink::default());
            let refused = harness
                .facade
                .register_input("s1", sink.clone())
                .await
                .expect_err("register is refused");
            assert_eq!(refused.code(), "not_ready");
            assert!(refused.to_string().contains(state.as_code()));

            for error in [
                harness
                    .facade
                    .inject_input("s1", Bytes::from_static(b"{}"))
                    .await
                    .expect_err("inject is refused"),
                harness
                    .facade
                    .publish_payload("s1", Bytes::from_static(b"{}"))
                    .await
                    .expect_err("publish is refused"),
                harness
                    .facade
                    .save_snapshot("s1", Bytes::from_static(b"snap"))
                    .await
                    .expect_err("save is refused"),
                harness
                    .facade
                    .append_event("s1", Bytes::from_static(b"event"))
                    .await
                    .expect_err("append is refused"),
            ] {
                assert_eq!(error.code(), "not_ready");
            }
            assert!(sink.delivered().is_empty());

            // A drain must still be able to release the session and read state.
            harness
                .facade
                .unregister_session("s1")
                .await
                .expect("teardown is allowed");
            assert!(harness
                .facade
                .load_snapshot("s1")
                .await
                .expect("reads are allowed")
                .is_none());
        }
    }

    #[test]
    fn lifecycle_names_are_stable_snake_case() {
        assert_eq!(
            serde_json::to_value(RuntimeLifecycle::ShuttingDown).expect("serialize lifecycle"),
            json!("shutting_down")
        );
        assert_eq!(
            serde_json::to_value(SessionLifecycle::AwaitingInput).expect("serialize phase"),
            json!("awaiting_input")
        );
        assert_eq!(
            serde_json::to_value(RuntimePort::Store).expect("serialize port"),
            json!("store")
        );
        assert_eq!(RuntimeLifecycle::Ready.as_code(), "ready");
    }

    #[tokio::test]
    async fn lifecycle_events_reuse_the_shared_stream_envelope() {
        let harness = ready();

        harness
            .facade
            .publish_lifecycle("s1", 7, SessionLifecycle::Running)
            .await
            .expect("publish lifecycle");

        let delivered = harness.transport.delivered.lock().expect("delivery lock");
        assert_eq!(delivered.len(), 1);
        let envelope: StreamEnvelope =
            serde_json::from_slice(&delivered[0].1).expect("canonical envelope");
        assert_eq!(envelope.schema_version, STREAM_SCHEMA_VERSION);
        assert_eq!(envelope.server_instance_id, "instance-a");
        assert_eq!(envelope.session_id, "s1");
        assert_eq!(envelope.sequence, 7);
        assert_eq!(envelope.payload, json!({"phase": "running"}));
    }

    #[test]
    fn failures_convert_to_the_shared_error_envelope() {
        let error =
            RuntimeCoreError::transport("publish_event", PortFailure("transport closed".into()));

        assert_eq!(error.code(), "transport_error");
        assert_eq!(
            error.to_envelope(),
            ErrorEnvelope::new("transport_error", error.to_string())
        );
        assert_eq!(
            serde_json::to_value(error.to_envelope()).expect("serialize envelope"),
            json!({
                "error": {
                    "code": "transport_error",
                    "message": "transport port failed during publish_event: transport closed",
                }
            })
        );
    }
}
