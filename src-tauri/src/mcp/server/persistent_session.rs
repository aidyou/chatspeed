//! Persistent Session Manager for Streamable HTTP transport.
//!
//! This module provides a `SessionManager` implementation that persists sessions
//! to disk using the `sled` embedded database. This ensures that sessions
//! can survive application restarts.
//!
//! It is heavily based on the `rmcp`'s `LocalSessionManager`, but adds a persistence
//! layer with `sled` and session expiration logic.

use crate::constants::STORE_DIR;
use anyhow::Context;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures::Stream;
use rmcp::{
    model::{
        CancelledNotificationParam, ClientJsonRpcMessage, ClientNotification, ClientRequest,
        JsonRpcNotification, JsonRpcRequest, Notification, ProgressNotificationParam,
        ProgressToken, RequestId, ServerJsonRpcMessage, ServerNotification,
    },
    service::serve_directly,
    transport::{
        common::server_side_http::{session_id, ServerSseMessage, SessionId},
        streamable_http_server::SessionManager,
        worker::{Worker, WorkerContext, WorkerQuitReason, WorkerSendRequest, WorkerTransport},
    },
    RoleServer,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    marker::PhantomData,
    num::ParseIntError,
    sync::Arc,
    time::Duration,
};
use thiserror::Error;
use tokio::sync::{
    mpsc::{Receiver, Sender},
    oneshot, RwLock,
};
use tokio_stream::wrappers::ReceiverStream;

const SESSION_LIFETIME_DAYS: i64 = 7;
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60 * 60); // 1 hour

//======================================================
// Session Data and Manager Definition
//======================================================
/// Data associated with a session, stored in the database.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionData {
    /// The time when the session expires.
    pub expires_at: DateTime<Utc>,
}

/// A `SessionManager` that persists sessions to disk using `sled`.
pub struct PersistentSessionManager<S: rmcp::Service<RoleServer>> {
    /// In-memory cache of active session handles.
    sessions: Arc<RwLock<HashMap<SessionId, LocalSessionHandle>>>,
    /// The `sled` database for persistence.
    db: sled::Db,
    /// Configuration for new sessions.
    session_config: SessionConfig,
    /// Factory to create new service instances.
    service_factory: Arc<dyn Fn() -> Result<S, std::io::Error> + Send + Sync>,
    /// Phantom data to hold the service type.
    _phantom: PhantomData<S>,
}

impl<S: rmcp::Service<RoleServer>> Clone for PersistentSessionManager<S> {
    fn clone(&self) -> Self {
        Self {
            sessions: self.sessions.clone(),
            db: self.db.clone(),
            session_config: self.session_config.clone(),
            service_factory: self.service_factory.clone(),
            _phantom: PhantomData,
        }
    }
}

//======================================================
// Error Types
//======================================================
#[derive(Debug, Error)]
pub enum PersistentSessionManagerError {
    #[error("Session not found: {0}")]
    SessionNotFound(SessionId),
    #[error("Session error: {0}")]
    SessionError(#[from] SessionError),
    #[error("Invalid event id: {0}")]
    InvalidEventId(#[from] EventIdParseError),
    #[error("Sled database error: {0}")]
    SledError(#[from] sled::Error),
    #[error("Failed to (de)serialize session data: {0}")]
    SerializationError(#[from] serde_json::Error),
    #[error("Failed to create service instance: {0}")]
    ServiceCreationError(std::io::Error),
}

//======================================================
// PersistentSessionManager Implementation
//======================================================
impl<S> PersistentSessionManager<S>
where
    S: rmcp::Service<RoleServer> + Send + 'static,
{
    /// Creates a new `PersistentSessionManager`.
    pub fn new(
        service_factory: Arc<dyn Fn() -> Result<S, std::io::Error> + Send + Sync>,
    ) -> anyhow::Result<Self> {
        let store_dir = STORE_DIR.read();
        let db_path = store_dir.join("mcp_sessions");
        let db = sled::open(&db_path)
            .with_context(|| format!("Failed to open sled database at {:?}", db_path))?;

        let manager = Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            db,
            session_config: SessionConfig::default(),
            service_factory,
            _phantom: PhantomData,
        };

        manager.spawn_cleanup_task();
        Ok(manager)
    }

    fn spawn_cleanup_task(&self) {
        let db = self.db.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(CLEANUP_INTERVAL).await;
                let now = Utc::now();

                for item in db.iter() {
                    if let Ok((session_id_bytes, session_data_bytes)) = item {
                        if let Ok(session_data) =
                            serde_json::from_slice::<SessionData>(&session_data_bytes)
                        {
                            if session_data.expires_at < now {
                                let _ = db.remove(&session_id_bytes);
                            }
                        }
                    }
                }
            }
        });
    }

    async fn rehydrate_session(
        &self,
        id: &SessionId,
    ) -> Result<LocalSessionHandle, PersistentSessionManagerError> {
        let (handle, worker) = create_local_session(id.clone(), self.session_config.clone(), true);
        let transport = WorkerTransport::spawn(worker);

        let service = (self.service_factory)()
            .map_err(PersistentSessionManagerError::ServiceCreationError)?;

        let session_manager_clone = self.clone();
        let session_id_clone = id.clone();

        tokio::spawn(async move {
            // Use serve_directly to skip the initialization check
            let running_service = serve_directly(
                service, transport,
                None, // We don't have peer_info for a rehydrated session, which is fine.
            );

            if let Err(e) = running_service.waiting().await {
                log::error!(
                    "Rehydrated session service for {} failed: {:?}",
                    session_id_clone,
                    e
                );
            }

            // Clean up the session from the in-memory map upon termination.
            let _ = session_manager_clone
                .sessions
                .write()
                .await
                .remove(&session_id_clone);
        });

        self.sessions
            .write()
            .await
            .insert(id.clone(), handle.clone());
        Ok(handle)
    }

    async fn get_or_rehydrate_handle(
        &self,
        id: &SessionId,
    ) -> Result<LocalSessionHandle, PersistentSessionManagerError> {
        let sessions = self.sessions.read().await;
        if let Some(handle) = sessions.get(id) {
            return Ok(handle.clone());
        }
        drop(sessions);

        if self.check_db_session(id).await? {
            self.rehydrate_session(id).await
        } else {
            Err(PersistentSessionManagerError::SessionNotFound(id.clone()))
        }
    }

    async fn check_db_session(
        &self,
        id: &SessionId,
    ) -> Result<bool, PersistentSessionManagerError> {
        match self.db.get(id.as_ref())? {
            Some(session_data_bytes) => {
                let session_data: SessionData = serde_json::from_slice(&session_data_bytes)?;
                if session_data.expires_at > Utc::now() {
                    Ok(true)
                } else {
                    self.db.remove(id.as_ref())?;
                    Ok(false)
                }
            }
            None => Ok(false),
        }
    }
}

//======================================================
// SessionManager Trait Implementation
//======================================================
impl<S> SessionManager for PersistentSessionManager<S>
where
    S: rmcp::Service<RoleServer> + Send + 'static,
{
    type Error = PersistentSessionManagerError;
    type Transport = WorkerTransport<LocalSessionWorker>;

    async fn create_session(&self) -> Result<(SessionId, Self::Transport), Self::Error> {
        let (handle, worker) =
            create_local_session(session_id(), self.session_config.clone(), false);
        let id = handle.id().clone();

        let session_data = SessionData {
            expires_at: Utc::now() + ChronoDuration::days(SESSION_LIFETIME_DAYS),
        };
        let session_data_bytes = serde_json::to_vec(&session_data)?;
        self.db.insert(id.as_ref(), session_data_bytes)?;

        self.sessions.write().await.insert(id.clone(), handle);

        Ok((id, WorkerTransport::spawn(worker)))
    }

    async fn initialize_session(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<ServerJsonRpcMessage, Self::Error> {
        let handle = self.get_or_rehydrate_handle(id).await?;
        handle.initialize(message).await.map_err(Into::into)
    }

    async fn close_session(&self, id: &SessionId) -> Result<(), Self::Error> {
        if let Some(handle) = self.sessions.write().await.remove(id) {
            handle.close().await?;
        }
        self.db.remove(id.as_ref())?;
        Ok(())
    }

    async fn has_session(&self, id: &SessionId) -> Result<bool, Self::Error> {
        if self.sessions.read().await.contains_key(id) {
            return Ok(true);
        }
        self.check_db_session(id).await
    }

    async fn create_stream(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + 'static, Self::Error> {
        let handle = self.get_or_rehydrate_handle(id).await?;
        let receiver = handle.establish_request_wise_channel().await?;
        handle
            .push_message(message, receiver.http_request_id)
            .await?;
        Ok(ReceiverStream::new(receiver.inner))
    }

    async fn create_standalone_stream(
        &self,
        id: &SessionId,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + 'static, Self::Error> {
        let handle = self.get_or_rehydrate_handle(id).await?;
        let receiver = handle.establish_common_channel().await?;
        Ok(ReceiverStream::new(receiver.inner))
    }

    async fn resume(
        &self,
        id: &SessionId,
        last_event_id: String,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + 'static, Self::Error> {
        let handle = self.get_or_rehydrate_handle(id).await?;
        let receiver = handle.resume(last_event_id.parse()?).await?;
        Ok(ReceiverStream::new(receiver.inner))
    }

    async fn accept_message(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<(), Self::Error> {
        let handle = self.get_or_rehydrate_handle(id).await?;
        handle.push_message(message, None).await?;
        Ok(())
    }
}

// =================================================================================
// Vendored Code from rmcp::transport::streamable_http_server::session::local
//
// The following code is copied from `rmcp` and made private to this module.
// It's the underlying machinery for running an in-memory session worker.
// =================================================================================

//======================================================
// EventId and Parsing
//======================================================
/// `<index>/request_id>`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventId {
    http_request_id: Option<HttpRequestId>,
    index: usize,
}

impl std::fmt::Display for EventId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.index)?;
        match &self.http_request_id {
            Some(http_request_id) => write!(f, "/{http_request_id}"),
            None => write!(f, ""),
        }
    }
}

#[derive(Debug, Clone, Error)]
pub enum EventIdParseError {
    #[error("Invalid index: {0}")]
    InvalidIndex(ParseIntError),
    #[error("Invalid numeric request id: {0}")]
    InvalidNumericRequestId(ParseIntError),
}

impl std::str::FromStr for EventId {
    type Err = EventIdParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((index, request_id)) = s.split_once('/') {
            let index = usize::from_str(index).map_err(EventIdParseError::InvalidIndex)?;
            let request_id =
                u64::from_str(request_id).map_err(EventIdParseError::InvalidNumericRequestId)?;
            Ok(EventId {
                http_request_id: Some(request_id),
                index,
            })
        } else {
            let index = usize::from_str(s).map_err(EventIdParseError::InvalidIndex)?;
            Ok(EventId {
                http_request_id: None,
                index,
            })
        }
    }
}

//======================================================
// Session Worker Internals
//======================================================
struct CachedTx {
    tx: Sender<ServerSseMessage>,
    cache: VecDeque<ServerSseMessage>,
    http_request_id: Option<HttpRequestId>,
    capacity: usize,
}

impl CachedTx {
    fn new(tx: Sender<ServerSseMessage>, http_request_id: Option<HttpRequestId>) -> Self {
        Self {
            cache: VecDeque::with_capacity(tx.capacity()),
            capacity: tx.capacity(),
            tx,
            http_request_id,
        }
    }
    fn new_common(tx: Sender<ServerSseMessage>) -> Self {
        Self::new(tx, None)
    }

    async fn send(&mut self, message: ServerJsonRpcMessage) {
        let index = self.cache.back().map_or(0, |m| {
            m.event_id
                .as_deref()
                .unwrap_or_default()
                .parse::<EventId>()
                .expect("valid event id")
                .index
                + 1
        });
        let event_id = EventId {
            http_request_id: self.http_request_id,
            index,
        };
        let message = ServerSseMessage {
            event_id: Some(event_id.to_string()),
            message: Arc::new(message),
        };
        if self.cache.len() >= self.capacity {
            self.cache.pop_front();
        }
        self.cache.push_back(message.clone());
        let _ = self.tx.send(message).await.inspect_err(|e| {
            let event_id = &e.0.event_id;
            log::trace!(
                "trying to send message in a closed session, event_id = {:?}",
                event_id
            )
        });
    }

    async fn sync(&mut self, index: usize) -> Result<(), SessionError> {
        let Some(front) = self.cache.front() else {
            return Ok(());
        };
        let front_event_id = front
            .event_id
            .as_deref()
            .unwrap_or_default()
            .parse::<EventId>()?;
        let sync_index = index.saturating_sub(front_event_id.index);
        if sync_index > self.cache.len() {
            return Err(SessionError::InvalidEventId);
        }
        for message in self.cache.iter().skip(sync_index) {
            let send_result = self.tx.send(message.clone()).await;
            if send_result.is_err() {
                let event_id: EventId = message.event_id.as_deref().unwrap_or_default().parse()?;
                return Err(SessionError::ChannelClosed(Some(event_id.index as u64)));
            }
        }
        Ok(())
    }
}

struct HttpRequestWise {
    resources: HashSet<ResourceKey>,
    tx: CachedTx,
}

type HttpRequestId = u64;
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum ResourceKey {
    McpRequestId(RequestId),
    ProgressToken(ProgressToken),
}

pub struct LocalSessionWorker {
    id: SessionId,
    next_http_request_id: HttpRequestId,
    tx_router: HashMap<HttpRequestId, HttpRequestWise>,
    resource_router: HashMap<ResourceKey, HttpRequestId>,
    common: CachedTx,
    event_rx: Receiver<SessionEvent>,
    session_config: SessionConfig,
    is_rehydrated: bool,
}

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("Invalid request id: {0}")]
    DuplicatedRequestId(HttpRequestId),
    #[error("Channel closed: {0:?}")]
    ChannelClosed(Option<HttpRequestId>),
    #[error("Cannot parse event id: {0}")]
    EventIdParseError(#[from] EventIdParseError),
    #[error("Session service terminated")]
    SessionServiceTerminated,
    #[error("Invalid event id")]
    InvalidEventId,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<SessionError> for std::io::Error {
    fn from(value: SessionError) -> Self {
        match value {
            SessionError::Io(io) => io,
            _ => std::io::Error::other(format!("Session error: {value}")),
        }
    }
}

enum OutboundChannel {
    RequestWise { id: HttpRequestId, close: bool },
    Common,
}
#[derive(Debug)]
pub struct StreamableHttpMessageReceiver {
    pub http_request_id: Option<HttpRequestId>,
    pub inner: Receiver<ServerSseMessage>,
}

impl LocalSessionWorker {
    fn unregister_resource(&mut self, resource: &ResourceKey) {
        if let Some(http_request_id) = self.resource_router.remove(resource) {
            log::trace!(
                "unregister resource, resource = {:?}, http_request_id = {}",
                resource,
                http_request_id
            );
            if let Some(channel) = self.tx_router.get_mut(&http_request_id) {
                if channel.resources.is_empty() || matches!(resource, ResourceKey::McpRequestId(_))
                {
                    log::debug!(
                        "close http request wise channel, http_request_id = {}",
                        http_request_id
                    );
                    if let Some(channel) = self.tx_router.remove(&http_request_id) {
                        for resource in channel.resources {
                            self.resource_router.remove(&resource);
                        }
                    }
                }
            } else {
                log::warn!(
                    "http request wise channel not found, http_request_id = {}",
                    http_request_id
                );
            }
        }
    }
    fn register_resource(&mut self, resource: ResourceKey, http_request_id: HttpRequestId) {
        log::trace!(
            "register resource, resource = {:?}, http_request_id = {}",
            resource,
            http_request_id
        );
        if let Some(channel) = self.tx_router.get_mut(&http_request_id) {
            channel.resources.insert(resource.clone());
            self.resource_router.insert(resource, http_request_id);
        }
    }
    fn register_request(
        &mut self,
        request: &JsonRpcRequest<ClientRequest>,
        http_request_id: HttpRequestId,
    ) {
        use rmcp::model::GetMeta;
        self.register_resource(
            ResourceKey::McpRequestId(request.id.clone()),
            http_request_id,
        );
        if let Some(progress_token) = request.request.get_meta().get_progress_token() {
            self.register_resource(
                ResourceKey::ProgressToken(progress_token.clone()),
                http_request_id,
            );
        }
    }
    fn catch_cancellation_notification(
        &mut self,
        notification: &JsonRpcNotification<ClientNotification>,
    ) {
        if let ClientNotification::CancelledNotification(n) = &notification.notification {
            let request_id = n.params.request_id.clone();
            let resource = ResourceKey::McpRequestId(request_id);
            self.unregister_resource(&resource);
        }
    }
    fn next_http_request_id(&mut self) -> HttpRequestId {
        let id = self.next_http_request_id;
        self.next_http_request_id = self.next_http_request_id.wrapping_add(1);
        id
    }
    async fn establish_request_wise_channel(
        &mut self,
    ) -> Result<StreamableHttpMessageReceiver, SessionError> {
        let http_request_id = self.next_http_request_id();
        let (tx, rx) = tokio::sync::mpsc::channel(self.session_config.channel_capacity);
        self.tx_router.insert(
            http_request_id,
            HttpRequestWise {
                resources: Default::default(),
                tx: CachedTx::new(tx, Some(http_request_id)),
            },
        );
        log::debug!(
            "establish new request wise channel, http_request_id = {}",
            http_request_id
        );
        Ok(StreamableHttpMessageReceiver {
            http_request_id: Some(http_request_id),
            inner: rx,
        })
    }
    fn resolve_outbound_channel(&self, message: &ServerJsonRpcMessage) -> OutboundChannel {
        match &message {
            ServerJsonRpcMessage::Request(_) => OutboundChannel::Common,
            ServerJsonRpcMessage::Notification(JsonRpcNotification {
                notification:
                    ServerNotification::ProgressNotification(Notification {
                        params: ProgressNotificationParam { progress_token, .. },
                        ..
                    }),
                ..
            }) => {
                let id = self
                    .resource_router
                    .get(&ResourceKey::ProgressToken(progress_token.clone()));

                if let Some(id) = id {
                    OutboundChannel::RequestWise {
                        id: *id,
                        close: false,
                    }
                } else {
                    OutboundChannel::Common
                }
            }
            ServerJsonRpcMessage::Notification(JsonRpcNotification {
                notification:
                    ServerNotification::CancelledNotification(Notification {
                        params: CancelledNotificationParam { request_id, .. },
                        ..
                    }),
                ..
            }) => {
                if let Some(id) = self
                    .resource_router
                    .get(&ResourceKey::McpRequestId(request_id.clone()))
                {
                    OutboundChannel::RequestWise {
                        id: *id,
                        close: false,
                    }
                } else {
                    OutboundChannel::Common
                }
            }
            ServerJsonRpcMessage::Notification(_) => OutboundChannel::Common,
            ServerJsonRpcMessage::Response(json_rpc_response) => {
                if let Some(id) = self
                    .resource_router
                    .get(&ResourceKey::McpRequestId(json_rpc_response.id.clone()))
                {
                    OutboundChannel::RequestWise {
                        id: *id,
                        close: false,
                    }
                } else {
                    OutboundChannel::Common
                }
            }
            ServerJsonRpcMessage::Error(json_rpc_error) => {
                if let Some(id) = self
                    .resource_router
                    .get(&ResourceKey::McpRequestId(json_rpc_error.id.clone()))
                {
                    OutboundChannel::RequestWise {
                        id: *id,
                        close: false,
                    }
                } else {
                    OutboundChannel::Common
                }
            }
        }
    }
    async fn handle_server_message(
        &mut self,
        message: ServerJsonRpcMessage,
    ) -> Result<(), SessionError> {
        let outbound_channel = self.resolve_outbound_channel(&message);
        match outbound_channel {
            OutboundChannel::RequestWise { id, close } => {
                if let Some(request_wise) = self.tx_router.get_mut(&id) {
                    request_wise.tx.send(message).await;
                    if close {
                        self.tx_router.remove(&id);
                    }
                } else {
                    return Err(SessionError::ChannelClosed(Some(id)));
                }
            }
            OutboundChannel::Common => self.common.send(message).await,
        }
        Ok(())
    }
    async fn resume(
        &mut self,
        last_event_id: EventId,
    ) -> Result<StreamableHttpMessageReceiver, SessionError> {
        match last_event_id.http_request_id {
            Some(http_request_id) => {
                let request_wise = self
                    .tx_router
                    .get_mut(&http_request_id)
                    .ok_or(SessionError::ChannelClosed(Some(http_request_id)))?;
                let channel = tokio::sync::mpsc::channel(self.session_config.channel_capacity);
                let (tx, rx) = channel;
                request_wise.tx.tx = tx;
                let index = last_event_id.index;
                request_wise.tx.sync(index).await?;
                Ok(StreamableHttpMessageReceiver {
                    http_request_id: Some(http_request_id),
                    inner: rx,
                })
            }
            None => {
                let channel = tokio::sync::mpsc::channel(self.session_config.channel_capacity);
                let (tx, rx) = channel;
                self.common.tx = tx;
                let index = last_event_id.index;
                self.common.sync(index).await?;
                Ok(StreamableHttpMessageReceiver {
                    http_request_id: None,
                    inner: rx,
                })
            }
        }
    }
}

//======================================================
// Session Events and Handle
//======================================================
#[derive(Debug)]
pub enum SessionEvent {
    ClientMessage {
        message: ClientJsonRpcMessage,
        http_request_id: Option<HttpRequestId>,
    },
    EstablishRequestWiseChannel {
        responder: oneshot::Sender<Result<StreamableHttpMessageReceiver, SessionError>>,
    },
    Resume {
        last_event_id: EventId,
        responder: oneshot::Sender<Result<StreamableHttpMessageReceiver, SessionError>>,
    },
    InitializeRequest {
        request: ClientJsonRpcMessage,
        responder: oneshot::Sender<Result<ServerJsonRpcMessage, SessionError>>,
    },
    Close,
}

#[derive(Debug, Clone)]
pub struct LocalSessionHandle {
    id: SessionId,
    event_tx: Sender<SessionEvent>,
}

impl LocalSessionHandle {
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    pub async fn close(&self) -> Result<(), SessionError> {
        self.event_tx
            .send(SessionEvent::Close)
            .await
            .map_err(|_| SessionError::SessionServiceTerminated)?;
        Ok(())
    }

    pub async fn push_message(
        &self,
        message: ClientJsonRpcMessage,
        http_request_id: Option<HttpRequestId>,
    ) -> Result<(), SessionError> {
        self.event_tx
            .send(SessionEvent::ClientMessage {
                message,
                http_request_id,
            })
            .await
            .map_err(|_| SessionError::SessionServiceTerminated)?;
        Ok(())
    }

    pub async fn establish_request_wise_channel(
        &self,
    ) -> Result<StreamableHttpMessageReceiver, SessionError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.event_tx
            .send(SessionEvent::EstablishRequestWiseChannel { responder: tx })
            .await
            .map_err(|_| SessionError::SessionServiceTerminated)?;
        rx.await
            .map_err(|_| SessionError::SessionServiceTerminated)?
    }

    pub async fn establish_common_channel(
        &self,
    ) -> Result<StreamableHttpMessageReceiver, SessionError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.event_tx
            .send(SessionEvent::Resume {
                last_event_id: EventId {
                    http_request_id: None,
                    index: 0,
                },
                responder: tx,
            })
            .await
            .map_err(|_| SessionError::SessionServiceTerminated)?;
        rx.await
            .map_err(|_| SessionError::SessionServiceTerminated)?
    }

    pub async fn resume(
        &self,
        last_event_id: EventId,
    ) -> Result<StreamableHttpMessageReceiver, SessionError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.event_tx
            .send(SessionEvent::Resume {
                last_event_id,
                responder: tx,
            })
            .await
            .map_err(|_| SessionError::SessionServiceTerminated)?;
        rx.await
            .map_err(|_| SessionError::SessionServiceTerminated)?
    }

    pub async fn initialize(
        &self,
        request: ClientJsonRpcMessage,
    ) -> Result<ServerJsonRpcMessage, SessionError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.event_tx
            .send(SessionEvent::InitializeRequest {
                request,
                responder: tx,
            })
            .await
            .map_err(|_| SessionError::SessionServiceTerminated)?;
        rx.await
            .map_err(|_| SessionError::SessionServiceTerminated)?
    }
}

//======================================================
// Worker Implementation
//======================================================
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Error)]
pub enum LocalSessionWorkerError {
    #[error("transport terminated")]
    TransportTerminated,
    #[error("unexpected message: {0:?}")]
    UnexpectedEvent(SessionEvent),
    #[error("fail to send initialize request {0}")]
    FailToSendInitializeRequest(SessionError),
    #[error("fail to handle message: {0}")]
    FailToHandleMessage(SessionError),
    #[error("keep alive timeout after {}ms", _0.as_millis())]
    KeepAliveTimeout(Duration),
    #[error("Transport closed")]
    TransportClosed,
    #[error("Tokio join error {0}")]
    TokioJoinError(#[from] tokio::task::JoinError),
}
impl Worker for LocalSessionWorker {
    type Error = LocalSessionWorkerError;
    type Role = RoleServer;
    fn err_closed() -> Self::Error {
        LocalSessionWorkerError::TransportClosed
    }
    fn err_join(e: tokio::task::JoinError) -> Self::Error {
        LocalSessionWorkerError::TokioJoinError(e)
    }
    fn config(&self) -> rmcp::transport::worker::WorkerConfig {
        rmcp::transport::worker::WorkerConfig {
            name: Some(format!("streamable-http-session-{}", self.id)),
            channel_buffer_capacity: self.session_config.channel_capacity,
        }
    }
    async fn run(
        mut self,
        mut context: WorkerContext<Self>,
    ) -> Result<(), WorkerQuitReason<Self::Error>> {
        enum InnerEvent {
            FromHttpService(SessionEvent),
            FromHandler(WorkerSendRequest<LocalSessionWorker>),
        }

        if !self.is_rehydrated {
            let evt = self.event_rx.recv().await.ok_or_else(|| {
                WorkerQuitReason::fatal(
                    LocalSessionWorkerError::TransportTerminated,
                    "get initialize request",
                )
            })?;
            let SessionEvent::InitializeRequest { request, responder } = evt else {
                return Err(WorkerQuitReason::fatal(
                    LocalSessionWorkerError::UnexpectedEvent(evt),
                    "get initialize request",
                ));
            };
            context.send_to_handler(request).await?;
            let send_initialize_response = context.recv_from_handler().await?;
            responder
                .send(Ok(send_initialize_response.message))
                .map_err(|_| {
                    WorkerQuitReason::fatal(
                        LocalSessionWorkerError::FailToSendInitializeRequest(
                            SessionError::SessionServiceTerminated,
                        ),
                        "send initialize response",
                    )
                })?;
            send_initialize_response
                .responder
                .send(Ok(()))
                .map_err(|_| WorkerQuitReason::HandlerTerminated)?;
        }

        let ct = context.cancellation_token.clone();
        let keep_alive = self.session_config.keep_alive.unwrap_or(Duration::MAX);
        loop {
            let keep_alive_timeout = tokio::time::sleep(keep_alive);
            let event = tokio::select! {
                event = self.event_rx.recv() => {
                    if let Some(event) = event {
                        InnerEvent::FromHttpService(event)
                    } else {
                        return Err(WorkerQuitReason::fatal(LocalSessionWorkerError::TransportTerminated, "waiting next session event"))
                    }
                },
                from_handler = context.recv_from_handler() => {
                    InnerEvent::FromHandler(from_handler?)
                }
                _ = ct.cancelled() => {
                    return Err(WorkerQuitReason::Cancelled)
                }
                _ = keep_alive_timeout => {
                    return Err(WorkerQuitReason::fatal(LocalSessionWorkerError::KeepAliveTimeout(keep_alive), "poll next session event"))
                }
            };
            match event {
                InnerEvent::FromHandler(WorkerSendRequest { message, responder }) => {
                    let to_unregister = match &message {
                        rmcp::model::JsonRpcMessage::Response(json_rpc_response) => {
                            let request_id = json_rpc_response.id.clone();
                            Some(ResourceKey::McpRequestId(request_id))
                        }
                        rmcp::model::JsonRpcMessage::Error(json_rpc_error) => {
                            let request_id = json_rpc_error.id.clone();
                            Some(ResourceKey::McpRequestId(request_id))
                        }
                        _ => None,
                    };
                    let handle_result = self
                        .handle_server_message(message)
                        .await
                        .map_err(LocalSessionWorkerError::FailToHandleMessage);
                    let _ = responder.send(handle_result).inspect_err(|error| {
                        log::warn!(
                            "failed to send message to http service handler, error = {:?}",
                            error
                        );
                    });
                    if let Some(to_unregister) = to_unregister {
                        self.unregister_resource(&to_unregister);
                    }
                }
                InnerEvent::FromHttpService(SessionEvent::ClientMessage {
                    message: json_rpc_message,
                    http_request_id,
                }) => {
                    match &json_rpc_message {
                        rmcp::model::JsonRpcMessage::Request(request) => {
                            if let Some(http_request_id) = http_request_id {
                                self.register_request(request, http_request_id)
                            }
                        }
                        rmcp::model::JsonRpcMessage::Notification(notification) => {
                            self.catch_cancellation_notification(notification)
                        }
                        _ => {}
                    }
                    context.send_to_handler(json_rpc_message).await?;
                }
                InnerEvent::FromHttpService(SessionEvent::EstablishRequestWiseChannel {
                    responder,
                }) => {
                    let handle_result = self.establish_request_wise_channel().await;
                    let _ = responder.send(handle_result);
                }
                InnerEvent::FromHttpService(SessionEvent::Resume {
                    last_event_id,
                    responder,
                }) => {
                    let handle_result = self.resume(last_event_id).await;
                    let _ = responder.send(handle_result);
                }
                InnerEvent::FromHttpService(SessionEvent::Close) => {
                    return Err(WorkerQuitReason::TransportClosed);
                }
                _ => {}
            }
        }
    }
}

//======================================================
// Session Config and Creation
//======================================================
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub channel_capacity: usize,
    pub keep_alive: Option<Duration>,
}

impl SessionConfig {
    pub const DEFAULT_CHANNEL_CAPACITY: usize = 16;
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            channel_capacity: Self::DEFAULT_CHANNEL_CAPACITY,
            keep_alive: None,
        }
    }
}

pub fn create_local_session(
    id: impl Into<SessionId>,
    config: SessionConfig,
    is_rehydrated: bool,
) -> (LocalSessionHandle, LocalSessionWorker) {
    let id = id.into();
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(config.channel_capacity);
    let (common_tx, _) = tokio::sync::mpsc::channel(config.channel_capacity);
    let common = CachedTx::new_common(common_tx);
    if !is_rehydrated {
        log::info!("create new session, session_id = {:?}", id);
    } else {
        log::info!("rehydrate session, session_id = {:?}", id);
    }
    let handle = LocalSessionHandle {
        event_tx,
        id: id.clone(),
    };
    let session_worker = LocalSessionWorker {
        next_http_request_id: 0,
        id,
        tx_router: HashMap::new(),
        resource_router: HashMap::new(),
        common,
        event_rx,
        session_config: config,
        is_rehydrated,
    };
    (handle, session_worker)
}
