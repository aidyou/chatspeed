//! Desktop adapters that route the existing Tauri agent commands to the
//! standalone runtime control plane.
//!
//! The runtime process owns the agent table, so the Tauri commands must stop
//! calling a local `MainStore` and reach the runtime through the typed
//! `RuntimeClient` the [`RuntimeSupervisor`] hands out, using the documented
//! `/control/v1/agents` routes only. This module owns the request/response
//! mapping so each command wrapper stays a thin translation of its Tauri wire
//! into one runtime call.
//!
//! There is deliberately no local fallback: when the supervisor holds no lease
//! the call fails instead of silently speaking to a second owner.
//!
//! The desktop's runtime client exposes a POST-only mutation surface, so the
//! runtime serves the update/delete handlers behind the POST `/update` and
//! `/delete` aliases as well as the RESTful `PUT`/`PATCH`/`DELETE` verbs. This
//! adapter uses those aliases; both map to the same typed handler.
//!
//! Requests use the `Agent`'s own serde shape (snake_case top level, camelCase
//! nested model configs); the runtime echoes the same shape, so the response
//! decodes straight back into `Agent` with no re-casing.

use serde::de::DeserializeOwned;
use serde_json::Value;

use chatspeed_runtime_client::{ClientError, RuntimeClient};

use crate::db::Agent;
use crate::runtime_client::{RuntimeSupervisor, RuntimeUnavailable};

/// Canonical control-plane route for the agent collection.
const AGENTS_ROUTE: &str = "/control/v1/agents";

/// Lists all agents through the runtime control plane.
pub async fn list_agents(supervisor: &RuntimeSupervisor) -> Result<Vec<Agent>, String> {
    let client = control_plane_client(supervisor).await?;
    let value = client.get(AGENTS_ROUTE).await.map_err(map_client_error)?;
    decode_response(value)
}

/// Gets one agent, mapping the runtime's stable 404 to `Ok(None)`.
pub async fn get_agent(supervisor: &RuntimeSupervisor, id: &str) -> Result<Option<Agent>, String> {
    let client = control_plane_client(supervisor).await?;
    let path = format!("{AGENTS_ROUTE}/{id}");
    match client.get(&path).await {
        Ok(value) => decode_response(value).map(Some),
        Err(ClientError::Server { status, .. }) if status == 404 => Ok(None),
        Err(error) => Err(map_client_error(error)),
    }
}

/// Creates an agent through the runtime control plane and returns its id.
pub async fn add_agent(supervisor: &RuntimeSupervisor, agent: Agent) -> Result<String, String> {
    let client = control_plane_client(supervisor).await?;
    let body = serde_json::to_value(&agent).map_err(|error| error.to_string())?;
    let response = client
        .post_with_idempotency(AGENTS_ROUTE, &body, &new_idempotency_key())
        .await
        .map_err(map_client_error)?;
    response_string(&response, "id")
}

/// Updates an agent through the runtime control plane.
pub async fn update_agent(supervisor: &RuntimeSupervisor, agent: Agent) -> Result<(), String> {
    let client = control_plane_client(supervisor).await?;
    let body = serde_json::to_value(&agent).map_err(|error| error.to_string())?;
    let path = format!("{AGENTS_ROUTE}/{}/update", agent.id);
    client
        .post_with_idempotency(&path, &body, &new_idempotency_key())
        .await
        .map_err(map_client_error)?;
    Ok(())
}

/// Deletes an agent through the runtime control plane.
pub async fn delete_agent(supervisor: &RuntimeSupervisor, id: &str) -> Result<(), String> {
    let client = control_plane_client(supervisor).await?;
    let path = format!("{AGENTS_ROUTE}/{id}/delete");
    // The path parameter is authoritative; the control plane ignores the body.
    client
        .post_with_idempotency(&path, &serde_json::json!({}), &new_idempotency_key())
        .await
        .map_err(map_client_error)?;
    Ok(())
}

/// Resolves the connected control-plane client, or fails when no lease is held.
async fn control_plane_client(supervisor: &RuntimeSupervisor) -> Result<RuntimeClient, String> {
    supervisor.client().await.map_err(map_runtime_error)
}

/// Fresh idempotency key for one command invocation.
fn new_idempotency_key() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Decodes a runtime agent response back into the canonical `Agent` type.
fn decode_response<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("Unexpected runtime response: {error}"))
}

/// Reads a required string field from a runtime response object.
fn response_string(response: &Value, field: &str) -> Result<String, String> {
    response
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("Runtime response is missing the `{field}` field"))
}

/// Maps a transport/domain client error to the Tauri string error.
///
/// A structured runtime error keeps its bare message so the Tauri wire stays as
/// close as possible to the previous command's string error; every other
/// failure keeps the client's classified (redacted) description.
fn map_client_error(error: ClientError) -> String {
    match error {
        ClientError::Server { message, .. } => message,
        other => other.to_string(),
    }
}

/// Maps a supervisor availability failure to the Tauri string error.
fn map_runtime_error(error: RuntimeUnavailable) -> String {
    error.to_string()
}
