//! SSE streaming for live workflow events.
//!
//! Events are versioned [`StreamEnvelope`]s published by the shared runtime
//! hub. The stream supports:
//! - `Last-Event-ID` (or `?cursor=`) replay from the per-session bounded ring;
//! - explicit `reset_required` events when the cursor is stale, foreign, or
//!   outside the replay window — the client must re-fetch snapshot + durable
//!   events and re-subscribe without a cursor;
//! - keepalive comments so idle sessions do not time out.
//!
//! Slow subscribers never back-pressure the executor: a lagging subscriber
//! observes an explicit lag and receives `reset_required`.

use super::server::ControlPlaneState;
use crate::workflow::react::client::hub::{SessionSubscription, StreamEnvelope, SubscribeError};
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::Stream;
use std::convert::Infallible;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_stream::wrappers::ReceiverStream;

/// Interval between SSE keepalive comments.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// Query parameters for the stream endpoint.
#[derive(Debug, Default, serde::Deserialize)]
pub struct StreamQuery {
    pub cursor: Option<String>,
}

fn envelope_event(envelope: &StreamEnvelope) -> Event {
    let data = serde_json::to_string(envelope).unwrap_or_else(|_| "{}".to_string());
    Event::default().id(envelope.cursor()).data(data)
}

fn reset_event(reason: &str) -> Event {
    let data = serde_json::json!({
        "schema_version": super::dto::PROTOCOL_VERSION,
        "error": { "code": "reset_required", "reason": reason },
    });
    Event::default()
        .event("reset_required")
        .data(data.to_string())
}

/// `GET /control/v1/workflows/{session_id}/stream`.
pub async fn stream_workflow_events(
    State(state): State<ControlPlaneState>,
    Path(session_id): Path<String>,
    Query(query): Query<StreamQuery>,
    headers: HeaderMap,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let broker = state.svc.gateway.broker();
    let after_cursor = headers
        .get("Last-Event-ID")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .or(query.cursor);
    let subscription = broker.subscribe(&session_id, after_cursor.as_deref()).await;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);

    tokio::spawn(pump_events(
        subscription,
        tx,
        KEEPALIVE_INTERVAL,
        state.shutdown.clone(),
    ));

    // Bound the stream by the server's shutdown broadcast: an open SSE stream must
    // end the moment the control plane is asked to stop, otherwise axum's graceful
    // shutdown would wait forever on a stream that never finishes and
    // `ControlPlaneHandle::wait` could never complete. `take_until` ends the stream
    // (closing the connection) as soon as the broadcast fires.
    let stream = ReceiverStream::new(rx);
    let stream =
        futures_util::StreamExt::take_until(stream, await_shutdown(state.shutdown.clone()));
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Resolves once the control plane's shutdown broadcast fires, or its sender is
/// dropped — both mean the server is stopping and the stream must end.
async fn await_shutdown(mut shutdown: tokio::sync::watch::Receiver<bool>) {
    let _ = shutdown.changed().await;
}

async fn pump_events(
    subscription: Result<SessionSubscription, SubscribeError>,
    tx: tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    keepalive: Duration,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let subscription = match subscription {
        Ok(subscription) => subscription,
        Err(SubscribeError::ResetRequired { reason }) => {
            let _ = tx.send(Ok(reset_event(reason))).await;
            return;
        }
    };

    for envelope in subscription.replay {
        if tx.send(Ok(envelope_event(&envelope))).await.is_err() {
            return;
        }
    }

    let mut rx = subscription.rx;
    let mut keepalive_interval = tokio::time::interval(keepalive);
    keepalive_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick fires immediately; skip it so we do not emit a keepalive
    // right after the replay.
    keepalive_interval.tick().await;

    loop {
        tokio::select! {
            received = rx.recv() => {
                match received {
                    Ok(envelope) => {
                        if tx.send(Ok(envelope_event(&envelope))).await.is_err() {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // The subscriber fell behind the bounded channel: the
                        // window can no longer be reconstructed losslessly.
                        let _ = tx.send(Ok(reset_event("subscriber_lagged"))).await;
                        return;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        // Session stream removed (e.g. cleanup after stop).
                        return;
                    }
                }
            }
            _ = keepalive_interval.tick() => {
                if tx.send(Ok(Event::default().comment("keepalive"))).await.is_err() {
                    return;
                }
            }
            _ = shutdown.changed() => {
                // The server is stopping: end the pump so it does not linger
                // holding a subscription while the connection is torn down.
                return;
            }
        }
    }
}
