//! Independent loopback `/control/v1` HTTP/JSON + SSE control plane.
//!
//! This server is deliberately separate from the legacy static-file router and
//! from ccproxy (INV-5): it binds its own `127.0.0.1:0` listener, uses its own
//! bearer token from the discovery document, and never shares routes, CORS or
//! credentials with the other HTTP surfaces.
//!
//! Startup is non-fatal for the desktop app: if the control plane cannot bind
//! or publish its discovery document, the failure is logged (without secrets)
//! and the desktop workflow keeps working (INV-8).

use super::auth;
use super::discovery::{self, ControlPlaneDiscovery, CONTROL_PLANE_HOST, CONTROL_PROTOCOL_VERSION};
use super::dto::{self, MetaResponse};
use super::sse;
use crate::workflow::react::application::{
    ApplicationError, WorkflowApplicationService, WorkflowCreateRequest, WorkflowEventsQuery,
    WorkflowStartRequest,
};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use lru::LruCache;
use rand::Rng;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

/// Maximum request body size accepted by the control plane.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Maximum number of completed idempotency results retained for replay.
const IDEMPOTENCY_CACHE_SIZE: usize = 1024;

/// A completed idempotent mutation result.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct IdempotencyDone {
    status: StatusCode,
    body: String,
}

/// A completed result retained for sequential replay.
#[derive(Clone)]
struct DoneEntry {
    body_hash: u64,
    result: IdempotencyDone,
}

/// An in-flight reservation.
///
/// The watch channel retains the result once published, so subscribers that
/// attach at any time — before or after publication — observe it. The
/// reservation lives outside the bounded replay cache and can never be
/// evicted while executing.
pub(crate) struct InFlightReservation {
    body_hash: u64,
    result: watch::Sender<Option<IdempotencyDone>>,
}

/// Reservation outcome for an idempotent mutation.
pub(crate) enum Reservation {
    /// This request must execute the mutation and publish the result.
    Execute(Arc<InFlightReservation>),
    /// An identical mutation is in flight; wait for its single result.
    Wait(watch::Receiver<Option<IdempotencyDone>>),
    /// A completed identical mutation: replay without executing.
    Replay(IdempotencyDone),
    /// Same key with a different body.
    Conflict,
}

/// Instance-local idempotency tracker.
///
/// In-flight reservations live in a dedicated map outside the bounded replay
/// cache, so they can never be evicted while executing. `reserve` and
/// `complete` are synchronous and therefore atomic with respect to each other:
/// no concurrent or subsequent caller can observe an intermediate state
/// between result publication and cache completion.
pub(crate) struct IdempotencyTracker {
    in_flight: std::sync::Mutex<HashMap<String, Arc<InFlightReservation>>>,
    done: std::sync::Mutex<LruCache<String, DoneEntry>>,
}

impl IdempotencyTracker {
    pub(crate) fn new(done_capacity: usize) -> Self {
        Self {
            in_flight: std::sync::Mutex::new(HashMap::new()),
            done: std::sync::Mutex::new(LruCache::new(
                NonZeroUsize::new(done_capacity).expect("non-zero"),
            )),
        }
    }

    /// Reserves the key or attaches to an existing identical reservation.
    ///
    /// Holds the in-flight lock across check-and-insert so two concurrent
    /// first-time requests cannot both reserve. The done cache is only read
    /// under that lock; `complete` never holds both locks at once, so the
    /// nesting order (`in_flight` → `done`) cannot deadlock.
    pub(crate) fn reserve(&self, key: &str, body_hash: u64) -> Reservation {
        let mut in_flight = self.in_flight.lock().unwrap();
        if let Some(existing) = in_flight.get(key).cloned() {
            if existing.body_hash != body_hash {
                return Reservation::Conflict;
            }
            return Reservation::Wait(existing.result.subscribe());
        }

        {
            let mut done = self.done.lock().unwrap();
            if let Some(entry) = done.get(key) {
                if entry.body_hash == body_hash {
                    return Reservation::Replay(entry.result.clone());
                }
                return Reservation::Conflict;
            }
        }

        let (result, _) = watch::channel(None);
        let reservation = Arc::new(InFlightReservation { body_hash, result });
        in_flight.insert(key.to_string(), reservation.clone());
        Reservation::Execute(reservation)
    }

    /// Publishes the result to the reservation. `send_replace` retains the
    /// value even when no receiver exists yet, so a duplicate that subscribes
    /// at any later moment (before release) observes it instead of hanging.
    pub(crate) fn publish(&self, reservation: &InFlightReservation, done: &IdempotencyDone) {
        reservation.result.send_replace(Some(done.clone()));
    }

    /// Stores the result for bounded sequential replay and releases the
    /// in-flight reservation.
    pub(crate) fn finish(
        &self,
        key: &str,
        reservation: &Arc<InFlightReservation>,
        done: IdempotencyDone,
    ) {
        self.done.lock().unwrap().put(
            key.to_string(),
            DoneEntry {
                body_hash: reservation.body_hash,
                result: done,
            },
        );
        self.release(key, reservation);
    }

    /// Publishes the result and completes the reservation. Both steps are
    /// synchronous, so no concurrent or subsequent caller can observe an
    /// intermediate state between publication and cache completion.
    pub(crate) fn complete(
        &self,
        key: &str,
        reservation: &Arc<InFlightReservation>,
        done: IdempotencyDone,
    ) {
        self.publish(reservation, &done);
        self.finish(key, reservation, done);
    }

    /// Releases an in-flight reservation without a result (executor failure or
    /// panic). Waiters observe `None` and map it to a stable internal error
    /// instead of hanging. `send_replace` retains the `None` so late
    /// subscribers also observe the terminal state.
    pub(crate) fn abort(&self, key: &str, reservation: &Arc<InFlightReservation>) {
        reservation.result.send_replace(None);
        self.release(key, reservation);
    }

    fn release(&self, key: &str, reservation: &Arc<InFlightReservation>) {
        let mut in_flight = self.in_flight.lock().unwrap();
        if in_flight
            .get(key)
            .map(|current| Arc::ptr_eq(current, reservation))
            .unwrap_or(false)
        {
            in_flight.remove(key);
        }
    }
}

/// Aborts the in-flight reservation on early return or panic so waiters
/// observe a stable failure instead of hanging.
struct InFlightGuard {
    tracker: Arc<IdempotencyTracker>,
    key: String,
    reservation: Arc<InFlightReservation>,
    defused: bool,
}

impl InFlightGuard {
    fn defuse(mut self) {
        self.defused = true;
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if !self.defused {
            self.tracker.abort(&self.key, &self.reservation);
        }
    }
}

/// Shared router state.
#[derive(Clone)]
pub struct ControlPlaneState {
    pub svc: Arc<WorkflowApplicationService>,
    pub token: Arc<String>,
    pub server_instance_id: Arc<String>,
    pub(crate) idempotency: Arc<IdempotencyTracker>,
}

/// Handle for a running control-plane server.
#[derive(Clone)]
pub struct ControlPlaneHandle {
    pub port: u16,
    pub server_instance_id: String,
    shutdown: tokio::sync::watch::Sender<bool>,
}

/// The active control-plane handle, retained so the desktop app can request a
/// graceful shutdown (which also removes this instance's discovery document).
static ACTIVE_HANDLE: std::sync::Mutex<Option<ControlPlaneHandle>> = std::sync::Mutex::new(None);

/// Requests graceful shutdown of the active control plane, if any.
pub fn request_shutdown() {
    if let Some(handle) = ACTIVE_HANDLE.lock().unwrap().take() {
        handle.shutdown();
    }
}

impl ControlPlaneHandle {
    /// Requests a graceful shutdown and removes the discovery document when it
    /// still belongs to this instance.
    pub fn shutdown(&self) {
        let _ = self.shutdown.send(true);
    }
}

fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn now_timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("unix-{}", seconds)
}

/// Starts the control plane on an ephemeral loopback port and publishes the
/// discovery document.
pub async fn start(svc: Arc<WorkflowApplicationService>) -> Result<ControlPlaneHandle, String> {
    let token = Arc::new(generate_token());
    let server_instance_id = Arc::new(svc.gateway.broker().server_instance_id().to_string());

    let state = ControlPlaneState {
        svc,
        token: token.clone(),
        server_instance_id: server_instance_id.clone(),
        idempotency: Arc::new(IdempotencyTracker::new(IDEMPOTENCY_CACHE_SIZE)),
    };

    let router = build_router(state);

    let listener = tokio::net::TcpListener::bind((CONTROL_PLANE_HOST, 0))
        .await
        .map_err(|e| format!("failed to bind control plane listener: {}", e))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();

    let discovery_document = ControlPlaneDiscovery {
        protocol_version: CONTROL_PROTOCOL_VERSION.to_string(),
        server_instance_id: (*server_instance_id).clone(),
        pid: std::process::id(),
        host: CONTROL_PLANE_HOST.to_string(),
        port,
        token: (*token).clone(),
        started_at: now_timestamp(),
    };
    discovery::write_discovery(&discovery_document)?;

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    let cleanup_instance_id = (*server_instance_id).clone();
    tokio::spawn(async move {
        let server = axum::serve(listener, router).with_graceful_shutdown(async move {
            let _ = shutdown_rx.changed().await;
            log::info!("[ControlPlane] Shutdown signal received");
        });
        if let Err(error) = server.await {
            log::warn!("[ControlPlane] Server terminated with error: {}", error);
        }
        discovery::remove_discovery_if_instance(&cleanup_instance_id);
        log::info!("[ControlPlane] Server stopped");
    });

    log::info!(
        "[ControlPlane] Listening on {}:{} (instance {}, pid {})",
        CONTROL_PLANE_HOST,
        port,
        server_instance_id,
        std::process::id()
    );

    let handle = ControlPlaneHandle {
        port,
        server_instance_id: (*server_instance_id).clone(),
        shutdown: shutdown_tx,
    };
    *ACTIVE_HANDLE.lock().unwrap() = Some(handle.clone());
    Ok(handle)
}

// (Token material is only cloned into the discovery document and the router
// state; it never enters log paths.)

fn build_router(state: ControlPlaneState) -> Router {
    Router::new()
        .route("/control/v1/meta", get(meta))
        .route("/control/v1/agents", get(list_agents))
        .route("/control/v1/agents/{agent_id}", get(get_agent))
        .route(
            "/control/v1/workflows",
            get(list_workflows).post(create_workflow),
        )
        .route("/control/v1/workflows/{session_id}", get(get_workflow))
        .route(
            "/control/v1/workflows/{session_id}/start",
            post(start_workflow),
        )
        .route(
            "/control/v1/workflows/{session_id}/signal",
            post(signal_workflow),
        )
        .route(
            "/control/v1/workflows/{session_id}/stop",
            post(stop_workflow),
        )
        .route(
            "/control/v1/workflows/{session_id}/events",
            get(list_events),
        )
        .route(
            "/control/v1/workflows/{session_id}/stream",
            get(sse::stream_workflow_events),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_bearer,
        ))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

async fn meta(State(state): State<ControlPlaneState>) -> Json<MetaResponse> {
    Json(MetaResponse {
        service: "chatspeed-workflow-control-plane",
        protocol_version: dto::PROTOCOL_VERSION,
        schema_version: crate::workflow::react::client::hub::STREAM_SCHEMA_VERSION,
        server_instance_id: (*state.server_instance_id).clone(),
        pid: std::process::id(),
    })
}

async fn list_agents(State(state): State<ControlPlaneState>) -> Response {
    match state.svc.agent_list().await {
        Ok(agents) => snake_json_response(serde_json::to_value(&agents)),
        Err(error) => dto::application_error_response(&error),
    }
}

async fn get_agent(
    State(state): State<ControlPlaneState>,
    Path(agent_id): Path<String>,
) -> Response {
    match state.svc.agent_get(&agent_id).await {
        Ok(Some(agent)) => snake_json_response(serde_json::to_value(&agent)),
        Ok(None) => dto::application_error_response(&ApplicationError::not_found(format!(
            "Agent {} not found",
            agent_id
        ))),
        Err(error) => dto::application_error_response(&error),
    }
}

async fn list_workflows(State(state): State<ControlPlaneState>) -> Response {
    match state.svc.workflow_list().await {
        Ok(workflows) => snake_json_response(serde_json::to_value(&workflows)),
        Err(error) => dto::application_error_response(&error),
    }
}

async fn create_workflow(
    State(state): State<ControlPlaneState>,
    headers: HeaderMap,
    body: String,
) -> Response {
    with_idempotency(&state, &headers, &body, |state, body| async move {
        let request: WorkflowCreateRequest = match serde_json::from_str(&body) {
            Ok(request) => request,
            Err(error) => {
                return dto::error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_input",
                    format!("Invalid create request: {}", error),
                );
            }
        };
        match state.svc.create_workflow(request).await {
            Ok(session_id) => (
                StatusCode::CREATED,
                Json(serde_json::json!({ "session_id": session_id })),
            )
                .into_response(),
            Err(error) => dto::application_error_response(&error),
        }
    })
    .await
}

async fn get_workflow(
    State(state): State<ControlPlaneState>,
    Path(session_id): Path<String>,
) -> Response {
    match state.svc.workflow_snapshot(&session_id).await {
        Ok(snapshot) => snake_json_response(Ok(snapshot)),
        Err(error) => dto::application_error_response(&error),
    }
}

async fn start_workflow(
    State(state): State<ControlPlaneState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    with_idempotency(&state, &headers, &body, |state, body| async move {
        let mut request: WorkflowStartRequest = match serde_json::from_str(&body) {
            Ok(request) => request,
            Err(error) => {
                return dto::error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_input",
                    format!("Invalid start request: {}", error),
                );
            }
        };
        // The path parameter is authoritative for the target session.
        request.session_id = session_id;
        match state.svc.workflow_start(request).await {
            Ok(result) => (
                StatusCode::OK,
                Json(serde_json::json!({ "session_id": result })),
            )
                .into_response(),
            Err(error) => dto::application_error_response(&error),
        }
    })
    .await
}

async fn signal_workflow(
    State(state): State<ControlPlaneState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    with_idempotency(&state, &headers, &body, |state, body| async move {
        // The signal body is forwarded verbatim; typed signal validation stays
        // in the application service (INV-3).
        match state.svc.workflow_signal(&session_id, body).await {
            Ok(result) => (
                StatusCode::OK,
                Json(serde_json::json!({ "session_id": session_id, "result": result })),
            )
                .into_response(),
            Err(error) => dto::application_error_response(&error),
        }
    })
    .await
}

async fn stop_workflow(
    State(state): State<ControlPlaneState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    with_idempotency(&state, &headers, &body, |state, _body| async move {
        match state.svc.workflow_stop(&session_id).await {
            Ok(()) => (
                StatusCode::OK,
                Json(serde_json::json!({ "session_id": session_id, "stopped": true })),
            )
                .into_response(),
            Err(error) => dto::application_error_response(&error),
        }
    })
    .await
}

async fn list_events(
    State(state): State<ControlPlaneState>,
    Path(session_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let after = match params.get("after") {
        None => None,
        Some(raw) => match raw.parse::<i64>() {
            Ok(value) => Some(value),
            Err(_) => {
                return dto::error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_input",
                    "The 'after' parameter must be a durable event ID (integer)".to_string(),
                );
            }
        },
    };
    let limit = match params.get("limit") {
        None => None,
        Some(raw) => match raw.parse::<u32>() {
            Ok(value) => Some(value),
            Err(_) => {
                return dto::error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_input",
                    "The 'limit' parameter must be a positive integer".to_string(),
                );
            }
        },
    };

    let query = WorkflowEventsQuery {
        session_id,
        after,
        limit,
    };
    match state.svc.workflow_events(query).await {
        Ok(events) => snake_json_response(serde_json::to_value(&events)),
        Err(error) => dto::application_error_response(&error),
    }
}

/// Serializes a value and normalizes its keys to the HTTP snake_case wire.
fn snake_json_response(value: Result<serde_json::Value, serde_json::Error>) -> Response {
    match value {
        Ok(value) => Json(dto::to_snake_case_keys(value)).into_response(),
        Err(error) => dto::error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            format!("Failed to serialize response: {}", error),
        ),
    }
}

/// Executes a mutation with instance-local idempotency semantics:
/// - no `Idempotency-Key` header: execute directly;
/// - same key + same body: replay the stored response (no double effect);
/// - same key + different body: `409 conflict`.
async fn with_idempotency<F, Fut, T>(
    state: &ControlPlaneState,
    headers: &HeaderMap,
    body: &str,
    execute: F,
) -> Response
where
    F: FnOnce(ControlPlaneState, String) -> Fut,
    Fut: std::future::Future<Output = T>,
    T: IntoResponse,
{
    let key = match headers.get("Idempotency-Key").and_then(|v| v.to_str().ok()) {
        Some(key) if !key.is_empty() => key.to_string(),
        _ => {
            return execute(state.clone(), body.to_string())
                .await
                .into_response();
        }
    };

    let body_hash = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        body.hash(&mut hasher);
        hasher.finish()
    };

    // Reserve the key atomically: concurrent identical requests either replay
    // a finished result, wait for the single in-flight execution, or conflict
    // on a different body. In-flight reservations live outside the bounded
    // replay cache and the watch channel retains the published result, so no
    // caller can hang and the mutation never executes twice per key.
    match state.idempotency.reserve(&key, body_hash) {
        Reservation::Replay(done) => replay_response(done.status, done.body),
        Reservation::Conflict => dto::error_response(
            StatusCode::CONFLICT,
            "idempotency_key_conflict",
            "The Idempotency-Key was already used with a different request body".to_string(),
        ),
        Reservation::Wait(mut rx) => {
            // A subscriber attaching after publication observes the retained
            // result immediately; otherwise wait for the single publication.
            let done = {
                let retained = rx.borrow().clone();
                if let Some(done) = retained {
                    done
                } else if rx.changed().await.is_err() {
                    return idempotency_failed_response();
                } else {
                    match rx.borrow().clone() {
                        Some(done) => done,
                        None => return idempotency_failed_response(),
                    }
                }
            };
            replay_response(done.status, done.body)
        }
        Reservation::Execute(reservation) => {
            // Abort on early return or panic so waiters never hang.
            let guard = InFlightGuard {
                tracker: state.idempotency.clone(),
                key: key.clone(),
                reservation: reservation.clone(),
                defused: false,
            };

            let response = execute(state.clone(), body.to_string())
                .await
                .into_response();
            let status = response.status();
            let stored = match axum::body::to_bytes(response.into_body(), MAX_BODY_BYTES).await {
                Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
                Err(_) => String::new(),
            };
            let done = IdempotencyDone {
                status,
                body: stored,
            };

            state.idempotency.complete(&key, &reservation, done.clone());
            guard.defuse();

            replay_response(done.status, done.body)
        }
    }
}

/// Stable failure for callers waiting on an idempotent mutation that
/// terminated without a result.
fn idempotency_failed_response() -> Response {
    dto::error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "Idempotent mutation terminated without a result".to_string(),
    )
}

/// Rebuilds a stored mutation response, preserving the JSON content type.
fn replay_response(status: StatusCode, body: String) -> Response {
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::interaction::chat_completion::ChatState;
    use crate::db::MainStore;
    use crate::libs::tsid::TsidGenerator;
    use crate::libs::window_channels::WindowChannels;
    use crate::workflow::react::client::hub::WorkflowRuntimeHub;
    use crate::workflow::react::client::tauri::gateway::{EventSink, TauriGateway};
    use crate::workflow::react::events::{WorkflowEvent, WorkflowEventType};
    use crate::workflow::react::manager::WorkflowManager;
    use crate::workflow::react::orchestrator::{DefaultSubAgentFactory, SubAgentFactory};
    use crate::workflow::react::types::GatewayPayload;
    use std::sync::Mutex;

    /// Serializes env-dependent tests: `CHATSPEED_HOME` is process-global.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Holds the env lock for the duration of a test and clears the env var.
    struct EnvGuard(std::sync::MutexGuard<'static, ()>);

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            std::env::remove_var("CHATSPEED_HOME");
        }
    }

    struct RecordingSink(Mutex<Vec<(String, GatewayPayload)>>);

    impl EventSink for RecordingSink {
        fn emit(&self, event_name: &str, payload: &GatewayPayload) {
            self.0
                .lock()
                .unwrap()
                .push((event_name.to_string(), payload.clone()));
        }
    }

    struct TestApp {
        handle: ControlPlaneHandle,
        svc: Arc<WorkflowApplicationService>,
        store: Arc<MainStore>,
        _dir: tempfile::TempDir,
    }

    async fn spawn_test_app() -> (TestApp, EnvGuard) {
        let env = EnvGuard(ENV_LOCK.lock().unwrap());
        let dir = tempfile::tempdir().expect("temp dir");
        std::env::set_var("CHATSPEED_HOME", dir.path());
        let db_path = dir.path().join("control_plane_test.db");
        let store = Arc::new(MainStore::new(db_path).expect("store"));
        let chat_state = ChatState::new(Arc::new(WindowChannels::new()), None, store.clone());
        let tsid = Arc::new(TsidGenerator::new(1).expect("tsid"));
        let sink = Arc::new(RecordingSink(Mutex::new(Vec::new())));
        let tauri = Arc::new(TauriGateway::with_sink(sink));
        let hub = Arc::new(WorkflowRuntimeHub::new(tauri, "test-instance".to_string()));
        let manager = Arc::new(WorkflowManager::new());
        let factory: Arc<dyn SubAgentFactory> = Arc::new(DefaultSubAgentFactory {
            main_store: store.clone(),
            chat_state: chat_state.clone(),
            gateway: hub.clone(),
            workflow_manager: manager.clone(),
            app_data_dir: dir.path().to_path_buf(),
            tsid_generator: tsid.clone(),
        });
        let svc = Arc::new(WorkflowApplicationService::new(
            store.clone(),
            chat_state,
            tsid,
            hub,
            factory,
            manager,
            dir.path().to_path_buf(),
        ));
        let handle = start(svc.clone()).await.expect("control plane start");
        (
            TestApp {
                handle,
                svc,
                store,
                _dir: dir,
            },
            env,
        )
    }

    fn client() -> reqwest::Client {
        reqwest::Client::new()
    }

    fn auth_url(app: &TestApp, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", app.handle.port, path)
    }

    /// Reads the per-instance bearer token from the discovery document.
    fn auth_token(_app: &TestApp) -> String {
        discovery::read_discovery()
            .expect("discovery document")
            .token
    }

    async fn insert_agent(app: &TestApp, agent_id: &str) {
        let agent = crate::db::Agent::new(
            agent_id.to_string(),
            format!("Agent {}", agent_id),
            None,
            Some("primary".to_string()),
            None,
            String::new(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(false),
            Some(false),
            None,
        );
        app.store.add_agent(&agent).expect("insert agent");
    }

    #[tokio::test]
    async fn unauthorized_requests_are_rejected_with_stable_errors() {
        let (app, _env) = spawn_test_app().await;
        let client = client();

        // Missing token.
        let response = client
            .get(auth_url(&app, "/control/v1/meta"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], "unauthorized");

        // Wrong token.
        let response = client
            .get(auth_url(&app, "/control/v1/meta"))
            .header("Authorization", "Bearer wrong-token")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        // Browser Origin is rejected even with the correct token.
        let response = client
            .get(auth_url(&app, "/control/v1/meta"))
            .header("Authorization", format!("Bearer {}", auth_token(&app)))
            .header("Origin", "http://evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], "origin_forbidden");

        // Token in the URL is rejected.
        let response = client
            .get(format!(
                "{}?token={}",
                auth_url(&app, "/control/v1/meta"),
                auth_token(&app)
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], "token_in_url_forbidden");

        app.handle.shutdown();
    }

    #[tokio::test]
    async fn meta_reports_protocol_version_and_instance() {
        let (app, _env) = spawn_test_app().await;
        let client = client();

        let response = client
            .get(auth_url(&app, "/control/v1/meta"))
            .header("Authorization", format!("Bearer {}", auth_token(&app)))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["protocol_version"], "1");
        assert_eq!(body["server_instance_id"], "test-instance");
        assert_eq!(body["schema_version"], 1);
        assert_eq!(body["pid"], std::process::id());

        app.handle.shutdown();
    }

    #[tokio::test]
    async fn agent_endpoints_return_snake_case_and_stable_404() {
        let (app, _env) = spawn_test_app().await;
        insert_agent(&app, "agent-1").await;
        let client = client();
        let auth = format!("Bearer {}", auth_token(&app));

        let response = client
            .get(auth_url(&app, "/control/v1/agents"))
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body[0]["id"], "agent-1");
        // Agent fields are already snake_case; ensure no camelCase leaked.
        let serialized = serde_json::to_string(&body).unwrap();
        assert!(!serialized.contains("sandboxExecutionMode"));

        let response = client
            .get(auth_url(&app, "/control/v1/agents/missing-agent"))
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], "not_found");

        app.handle.shutdown();
    }

    #[tokio::test]
    async fn create_workflow_is_idempotent_per_key_and_conflicts_on_body_change() {
        let (app, _env) = spawn_test_app().await;
        insert_agent(&app, "agent-1").await;
        let client = client();
        let auth = format!("Bearer {}", auth_token(&app));
        let url = auth_url(&app, "/control/v1/workflows");
        let body = serde_json::json!({ "agent_id": "agent-1", "user_query": "hello" }).to_string();

        let response = client
            .post(&url)
            .header("Authorization", &auth)
            .header("Idempotency-Key", "key-1")
            .body(body.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let first: serde_json::Value = response.json().await.unwrap();
        let session_id = first["session_id"].as_str().unwrap().to_string();

        // Same key + same body: replayed, no second workflow created.
        let response = client
            .post(&url)
            .header("Authorization", &auth)
            .header("Idempotency-Key", "key-1")
            .body(body.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let second: serde_json::Value = response.json().await.unwrap();
        assert_eq!(second["session_id"], first["session_id"]);

        let workflows = app.store.list_workflows().expect("list workflows");
        assert_eq!(
            workflows.len(),
            1,
            "idempotent replay must not double-create"
        );

        // Same key + different body: conflict.
        let response = client
            .post(&url)
            .header("Authorization", &auth)
            .header("Idempotency-Key", "key-1")
            .body(
                serde_json::json!({ "agent_id": "agent-1", "user_query": "different" }).to_string(),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], "idempotency_key_conflict");

        assert!(!session_id.is_empty());
        app.handle.shutdown();
    }

    fn idempotency_done(status: StatusCode, body: &str) -> IdempotencyDone {
        IdempotencyDone {
            status,
            body: body.to_string(),
        }
    }

    #[test]
    fn idempotency_duplicate_after_completion_replays_without_reexecution() {
        let tracker = IdempotencyTracker::new(IDEMPOTENCY_CACHE_SIZE);
        let reservation = match tracker.reserve("k", 1) {
            Reservation::Execute(reservation) => reservation,
            _ => panic!("first reservation must execute"),
        };
        tracker.complete(
            "k",
            &reservation,
            idempotency_done(StatusCode::CREATED, "{\"a\":1}"),
        );

        // A duplicate reserving after completion must replay, never re-execute.
        match tracker.reserve("k", 1) {
            Reservation::Replay(done) => {
                assert_eq!(done.status, StatusCode::CREATED);
                assert_eq!(done.body, "{\"a\":1}");
            }
            _ => panic!("completed mutation must replay"),
        }
    }

    #[test]
    fn idempotency_in_flight_survives_done_cache_saturation() {
        let tracker = IdempotencyTracker::new(4);
        let reservation = match tracker.reserve("k", 1) {
            Reservation::Execute(reservation) => reservation,
            _ => panic!("first reservation must execute"),
        };

        // Saturate the bounded done cache with unrelated keys while "k" is
        // still in flight: the in-flight reservation lives outside the cache
        // and must not be evicted.
        for index in 0..8 {
            let key = format!("other-{index}");
            let other = match tracker.reserve(&key, 100 + index as u64) {
                Reservation::Execute(reservation) => reservation,
                _ => panic!("unrelated reservation must execute"),
            };
            tracker.complete(&key, &other, idempotency_done(StatusCode::OK, "{}"));
        }

        match tracker.reserve("k", 1) {
            Reservation::Wait(_) => {}
            _ => panic!("in-flight reservation must survive cache saturation"),
        }

        tracker.complete(
            "k",
            &reservation,
            idempotency_done(StatusCode::CREATED, "{\"ok\":1}"),
        );
        match tracker.reserve("k", 1) {
            Reservation::Replay(done) => assert_eq!(done.body, "{\"ok\":1}"),
            _ => panic!("must replay after completion"),
        }
    }

    #[tokio::test]
    async fn idempotency_waiter_observes_retained_result_without_hanging() {
        let tracker = Arc::new(IdempotencyTracker::new(IDEMPOTENCY_CACHE_SIZE));
        let reservation = match tracker.reserve("k", 1) {
            Reservation::Execute(reservation) => reservation,
            _ => panic!("first reservation must execute"),
        };
        // A duplicate subscribes while the mutation is in flight.
        let mut rx = match tracker.reserve("k", 1) {
            Reservation::Wait(rx) => rx,
            _ => panic!("duplicate must wait"),
        };

        // Completion publishes the retained result; the waiter must observe it
        // whether it reads before or after publication, and a late duplicate
        // must replay instead of re-executing.
        tracker.complete(
            "k",
            &reservation,
            idempotency_done(StatusCode::OK, "{\"done\":1}"),
        );

        let observed = {
            let retained = rx.borrow().clone();
            match retained {
                Some(done) => done,
                None => {
                    rx.changed().await.expect("result must be published");
                    rx.borrow().clone().expect("retained result")
                }
            }
        };
        assert_eq!(observed.body, "{\"done\":1}");

        match tracker.reserve("k", 1) {
            Reservation::Replay(done) => assert_eq!(done.body, "{\"done\":1}"),
            _ => panic!("post-completion duplicate must replay"),
        }
    }

    #[tokio::test]
    async fn idempotency_duplicate_subscribing_after_publication_before_release_gets_result() {
        let tracker = IdempotencyTracker::new(IDEMPOTENCY_CACHE_SIZE);
        let reservation = match tracker.reserve("k", 1) {
            Reservation::Execute(reservation) => reservation,
            _ => panic!("first reservation must execute"),
        };

        // Publish the result (retained even with zero receivers) but do NOT
        // finish yet: the reservation is still in flight. This is exactly the
        // window between publication and cache completion.
        let done = idempotency_done(StatusCode::CREATED, "{\"late\":1}");
        tracker.publish(&reservation, &done);

        // A duplicate reserving in this window must attach to the retained
        // result — never re-execute, never hang, never 500.
        let mut rx = match tracker.reserve("k", 1) {
            Reservation::Wait(rx) => rx,
            _ => panic!("duplicate must wait on the in-flight reservation"),
        };
        let observed = {
            let retained = rx.borrow().clone();
            match retained {
                Some(done) => done,
                None => {
                    rx.changed().await.expect("result must be retained");
                    rx.borrow().clone().expect("retained result")
                }
            }
        };
        assert_eq!(observed, done);

        // Finishing releases the reservation; later duplicates replay.
        tracker.finish("k", &reservation, done);
        match tracker.reserve("k", 1) {
            Reservation::Replay(replayed) => assert_eq!(replayed.body, "{\"late\":1}"),
            _ => panic!("post-release duplicate must replay"),
        }
    }

    #[tokio::test]
    async fn idempotency_abort_releases_waiters_and_frees_the_key() {
        let tracker = IdempotencyTracker::new(IDEMPOTENCY_CACHE_SIZE);
        let reservation = match tracker.reserve("k", 1) {
            Reservation::Execute(reservation) => reservation,
            _ => panic!("first reservation must execute"),
        };
        let mut rx = match tracker.reserve("k", 1) {
            Reservation::Wait(rx) => rx,
            _ => panic!("duplicate must wait"),
        };

        tracker.abort("k", &reservation);
        // The waiter observes the abort (None) instead of hanging.
        rx.changed().await.expect("abort must be observable");
        assert!(rx.borrow().clone().is_none());

        // The key is free again: a new reservation executes.
        match tracker.reserve("k", 1) {
            Reservation::Execute(_) => {}
            _ => panic!("aborted key must be reusable"),
        }
    }

    #[test]
    fn idempotency_different_body_conflicts_while_in_flight_and_after_completion() {
        let tracker = IdempotencyTracker::new(IDEMPOTENCY_CACHE_SIZE);
        let reservation = match tracker.reserve("k", 1) {
            Reservation::Execute(reservation) => reservation,
            _ => panic!("first reservation must execute"),
        };
        assert!(matches!(tracker.reserve("k", 2), Reservation::Conflict));
        tracker.complete("k", &reservation, idempotency_done(StatusCode::OK, "{}"));
        assert!(matches!(tracker.reserve("k", 2), Reservation::Conflict));
    }

    #[tokio::test]
    async fn concurrent_duplicate_idempotency_creates_exactly_one_workflow() {
        let (app, _env) = spawn_test_app().await;
        insert_agent(&app, "agent-1").await;
        let client = client();
        let auth = format!("Bearer {}", auth_token(&app));
        let url = auth_url(&app, "/control/v1/workflows");
        let body = serde_json::json!({ "agent_id": "agent-1", "user_query": "hello" }).to_string();

        // Fire concurrent identical mutations with the same key: only one may
        // execute; the others must wait for and replay the single result.
        let mut handles = Vec::new();
        for _ in 0..5 {
            let client = client.clone();
            let url = url.clone();
            let auth = auth.clone();
            let body = body.clone();
            handles.push(tokio::spawn(async move {
                client
                    .post(&url)
                    .header("Authorization", &auth)
                    .header("Idempotency-Key", "race-key")
                    .body(body)
                    .send()
                    .await
                    .unwrap()
            }));
        }

        let mut session_ids = std::collections::HashSet::new();
        for handle in handles {
            let response = handle.await.unwrap();
            assert_eq!(response.status(), StatusCode::CREATED);
            let value: serde_json::Value = response.json().await.unwrap();
            session_ids.insert(
                value["session_id"]
                    .as_str()
                    .expect("session_id in response")
                    .to_string(),
            );
        }

        assert_eq!(
            session_ids.len(),
            1,
            "all concurrent callers must observe the same session"
        );
        let workflows = app.store.list_workflows().expect("list workflows");
        assert_eq!(
            workflows.len(),
            1,
            "concurrent duplicate idempotency must not double-create"
        );

        app.handle.shutdown();
    }

    #[tokio::test]
    async fn events_endpoint_supports_after_and_limit_bounds() {
        let (app, _env) = spawn_test_app().await;
        insert_agent(&app, "agent-1").await;
        let client = client();
        let auth = format!("Bearer {}", auth_token(&app));

        let response = client
            .post(auth_url(&app, "/control/v1/workflows"))
            .header("Authorization", &auth)
            .body(serde_json::json!({ "agent_id": "agent-1" }).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let created: serde_json::Value = response.json().await.unwrap();
        let session_id = created["session_id"].as_str().unwrap().to_string();

        for index in 0..5 {
            app.store
                .append_workflow_event(&WorkflowEvent::new(
                    WorkflowEventType::WorkflowStarted,
                    session_id.clone(),
                    serde_json::json!({ "index": index }),
                ))
                .expect("append event");
        }

        let url = auth_url(
            &app,
            &format!("/control/v1/workflows/{}/events", session_id),
        );
        let response = client
            .get(&url)
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body.as_array().unwrap().len(), 5);
        let serialized = serde_json::to_string(&body).unwrap();
        assert!(serialized.contains("session_id"));
        assert!(!serialized.contains("sessionId"));

        // after + limit are honored.
        let response = client
            .get(format!("{}?after=2&limit=2", url))
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = response.json().await.unwrap();
        let ids: Vec<i64> = body
            .as_array()
            .unwrap()
            .iter()
            .map(|event| event["id"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, vec![3, 4]);

        // Invalid after is a stable 400.
        let response = client
            .get(format!("{}?after=not-a-number", url))
            .header("Authorization", &auth)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        app.handle.shutdown();
    }

    #[tokio::test]
    async fn sse_stream_replays_from_cursor_and_resets_on_stale_cursor() {
        let (app, _env) = spawn_test_app().await;
        let client = client();
        let auth = format!("Bearer {}", auth_token(&app));
        let broker = app.svc.gateway.broker();

        let first = broker
            .publish(
                "session-sse",
                GatewayPayload::Chunk {
                    content: "one".into(),
                },
            )
            .await;
        let _second = broker
            .publish(
                "session-sse",
                GatewayPayload::Chunk {
                    content: "two".into(),
                },
            )
            .await;

        // Replay after the first cursor. The stream stays open (keepalive), so
        // read chunks until the expected events arrive, then drop the client.
        let mut response = client
            .get(auth_url(&app, "/control/v1/workflows/session-sse/stream"))
            .header("Authorization", &auth)
            .header("Last-Event-ID", first.cursor())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .starts_with("text/event-stream"));
        let body = read_sse_until(&mut response, |buffer| {
            buffer.contains("\"content\":\"two\"") && buffer.contains("id: test-instance:1")
        })
        .await;
        assert!(body.contains("\"content\":\"two\""));
        assert!(body.contains("id: test-instance:1"));
        drop(response);

        // A foreign-instance cursor triggers reset_required and stream end.
        let mut response = client
            .get(auth_url(&app, "/control/v1/workflows/session-sse/stream"))
            .header("Authorization", &auth)
            .header("Last-Event-ID", "other-instance:0")
            .send()
            .await
            .unwrap();
        let body = read_sse_until(&mut response, |buffer| {
            buffer.contains("event: reset_required")
        })
        .await;
        assert!(body.contains("instance_mismatch"));

        app.handle.shutdown();
    }

    /// Reads SSE chunks until `predicate` matches the accumulated body or the
    /// stream ends, with a hard timeout so a broken stream fails the test.
    async fn read_sse_until(
        response: &mut reqwest::Response,
        predicate: impl Fn(&str) -> bool,
    ) -> String {
        let mut buffer = String::new();
        let deadline = std::time::Duration::from_secs(5);
        loop {
            let chunk = tokio::time::timeout(deadline, response.chunk())
                .await
                .expect("timed out waiting for SSE chunk")
                .expect("SSE read failed");
            match chunk {
                Some(bytes) => {
                    buffer.push_str(&String::from_utf8_lossy(&bytes));
                    if predicate(&buffer) {
                        return buffer;
                    }
                }
                None => return buffer,
            }
        }
    }

    #[tokio::test]
    async fn discovery_document_is_written_with_restricted_permissions_and_removed_on_shutdown() {
        let (app, _env) = spawn_test_app().await;
        let path = discovery::discovery_path();
        assert!(path.exists());
        let document = discovery::read_discovery().expect("discovery document");
        assert_eq!(document.protocol_version, "1");
        assert_eq!(document.server_instance_id, app.handle.server_instance_id);
        assert_eq!(document.port, app.handle.port);
        assert_eq!(document.host, "127.0.0.1");
        assert!(!document.token.is_empty());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let file_mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(file_mode & 0o777, 0o600);
            let dir_mode = std::fs::metadata(discovery::discovery_dir())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(dir_mode & 0o777, 0o700);
        }

        app.handle.shutdown();
        // Give the shutdown task a moment to clean up.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(!path.exists(), "discovery must be removed on shutdown");
    }
}
