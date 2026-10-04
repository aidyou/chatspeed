//! Desktop adapters that route the existing Tauri capability and MCP commands to
//! the standalone runtime control plane.
//!
//! The runtime process is the single owner of the capability journal, the MCP
//! runtime and the Agent Skill targets, so the Tauri commands must stop calling
//! a local `CapabilityApplicationService`/`ChatState`/`ToolManager` and reach the
//! runtime through the typed `RuntimeClient` the [`RuntimeSupervisor`] hands out,
//! using the documented `/control/v1` routes only. This module owns the
//! request/response mapping so each command wrapper stays a thin translation of
//! its Tauri wire into one runtime call.
//!
//! There is deliberately no local fallback and no second service: when the
//! supervisor holds no lease the call fails instead of silently speaking to a
//! second capability owner. Every mutation still travels through the canonical
//! control-plane idempotency header and the same durable journal, so retries
//! replay instead of applying twice.
//!
//! The MCP read/write routes reused here are the canonical capability routes plus
//! the explicit config-shaped MCP compatibility routes (`mcp-records`,
//! `mcp-record`, `mcp-update`, `mcp-tool-declarations`, `mcp-tool-status`); the
//! descriptor `mcp-install` route already accepts the desktop form's
//! `McpServerConfig` fields verbatim, so manual add enters the same install
//! operation as any other client.

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use chatspeed_runtime_client::{ClientError, RuntimeClient};

use crate::ai::traits::chat::MCPToolDeclaration;
use crate::capability::error::{code, CapabilityError};
use crate::capability::types::CapabilityOperation;
use crate::db::Mcp;
use crate::mcp::client::McpServerConfig;
use crate::runtime_client::{RuntimeSupervisor, RuntimeUnavailable};

/// Canonical capability read routes.
const SKILL_TARGETS_ROUTE: &str = "/control/v1/skill-targets";
const SKILLS_ROUTE: &str = "/control/v1/skills";
const MCP_SERVERS_ROUTE: &str = "/control/v1/mcp-servers";
const CAPABILITY_OPERATIONS_ROUTE: &str = "/control/v1/capability-operations";
const CAPABILITY_DOCTOR_ROUTE: &str = "/control/v1/capability-doctor";
const CAPABILITY_RECONCILE_ROUTE: &str = "/control/v1/capability-doctor/reconcile";

/// Canonical capability mutation routes.
const SKILL_CHECK_ROUTE: &str = "/control/v1/skill-check";
const SKILL_INSTALL_ROUTE: &str = "/control/v1/skill-install";
const SKILL_UNINSTALL_ROUTE: &str = "/control/v1/skill-uninstall";

/// Explicit MCP compatibility routes serving the desktop command wire.
const MCP_RECORDS_ROUTE: &str = "/control/v1/mcp-records";
const MCP_RECORD_ROUTE: &str = "/control/v1/mcp-record";
const MCP_INSTALL_ROUTE: &str = "/control/v1/mcp-install";
const MCP_UPDATE_ROUTE: &str = "/control/v1/mcp-update";
const MCP_UNINSTALL_ROUTE: &str = "/control/v1/mcp-uninstall";
const MCP_ENABLE_ROUTE: &str = "/control/v1/mcp-enable";
const MCP_DISABLE_ROUTE: &str = "/control/v1/mcp-disable";
const MCP_RESTART_ROUTE: &str = "/control/v1/mcp-restart";
const MCP_REFRESH_ROUTE: &str = "/control/v1/mcp-refresh";
const MCP_TOOL_DECLARATIONS_ROUTE: &str = "/control/v1/mcp-tool-declarations";
const MCP_TOOL_STATUS_ROUTE: &str = "/control/v1/mcp-tool-status";

/// Read-only client WebView capability bridge route (U-7).
const CLIENT_CAPABILITIES_ROUTE: &str = "/control/v1/client-capabilities";

/// A capability result, already carrying the stable wire code the Tauri
/// adapters branch on.
type CapResult<T> = Result<T, CapabilityError>;

// ---------------------------------------------------------------------------
// Capability reads and Skill mutations
// ---------------------------------------------------------------------------

/// The closed Skill install-target registry.
pub async fn skill_targets(supervisor: &RuntimeSupervisor) -> CapResult<Value> {
    get_value(supervisor, SKILL_TARGETS_ROUTE).await
}

/// The Agent Skill inventory.
pub async fn skill_inventory(supervisor: &RuntimeSupervisor) -> CapResult<Value> {
    get_value(supervisor, SKILLS_ROUTE).await
}

/// MCP servers with their desired, observed runtime and tools state.
pub async fn mcp_servers(supervisor: &RuntimeSupervisor) -> CapResult<Value> {
    get_value(supervisor, MCP_SERVERS_ROUTE).await
}

/// One durable capability operation.
pub async fn operation(
    supervisor: &RuntimeSupervisor,
    operation_id: &str,
) -> CapResult<CapabilityOperation> {
    let path = format!(
        "{CAPABILITY_OPERATIONS_ROUTE}/{}",
        encode_path_segment(operation_id)
    );
    decode(get_value(supervisor, &path).await?)
}

/// The capability doctor report.
pub async fn doctor(supervisor: &RuntimeSupervisor) -> CapResult<Value> {
    get_value(supervisor, CAPABILITY_DOCTOR_ROUTE).await
}

/// Converges the capability drift the durable evidence proves.
pub async fn reconcile(supervisor: &RuntimeSupervisor) -> CapResult<Value> {
    post_idempotent(
        supervisor,
        CAPABILITY_RECONCILE_ROUTE,
        &json!({}),
        &new_key(),
    )
    .await
}

/// Checks a Skill source with the non-LLM checker, without installing.
pub async fn skill_check(supervisor: &RuntimeSupervisor, source: &Value) -> CapResult<Value> {
    let client = control_plane_client(supervisor).await?;
    client
        .post(SKILL_CHECK_ROUTE, source)
        .await
        .map_err(map_client_error)
}

/// Installs a checked Skill into the selected targets.
///
/// The caller's idempotency key is forwarded verbatim, so the durable journal
/// replays a retried install instead of touching a target twice (AC-2).
pub async fn skill_install(
    supervisor: &RuntimeSupervisor,
    source: &Value,
    targets: &[String],
    idempotency_key: &str,
) -> CapResult<Value> {
    let body = json!({ "source": source, "targets": targets });
    post_idempotent(supervisor, SKILL_INSTALL_ROUTE, &body, idempotency_key).await
}

/// Uninstalls a Skill that ChatSpeed installed and still owns.
pub async fn skill_uninstall(
    supervisor: &RuntimeSupervisor,
    skill_name: &str,
    targets: &[String],
    idempotency_key: &str,
) -> CapResult<Value> {
    let body = json!({ "skill_name": skill_name, "targets": targets });
    post_idempotent(supervisor, SKILL_UNINSTALL_ROUTE, &body, idempotency_key).await
}

// ---------------------------------------------------------------------------
// MCP reads and mutations
// ---------------------------------------------------------------------------

/// The secret-free editable MCP records with the runtime status already overlaid.
pub async fn mcp_list_records(supervisor: &RuntimeSupervisor) -> CapResult<Vec<Mcp>> {
    decode(get_value(supervisor, MCP_RECORDS_ROUTE).await?)
}

/// One secret-free editable MCP record.
pub async fn mcp_get_record(supervisor: &RuntimeSupervisor, id: i64) -> CapResult<Mcp> {
    let path = format!("{MCP_RECORD_ROUTE}?id={id}");
    decode(get_value(supervisor, &path).await?)
}

/// Registers one MCP server, always disabled, then optionally enables it.
///
/// The desktop manual-add form edits a `McpServerConfig`; its fields are exactly
/// the strict descriptor fields, so it enters the canonical `mcp-install`
/// operation rather than a second install path. Installation performs no runtime
/// effect; enabling is a separate, separately auditable operation (AC-9).
pub async fn mcp_add_server(
    supervisor: &RuntimeSupervisor,
    name: &str,
    description: &str,
    config: &McpServerConfig,
    disabled: bool,
) -> CapResult<Mcp> {
    let descriptor = install_descriptor(name, description, config)?;
    let installed = post_idempotent(supervisor, MCP_INSTALL_ROUTE, &descriptor, &new_key()).await?;
    let id = mutated_id(&installed)?;
    if !disabled {
        // The record is intentionally kept when the start fails: the user can see
        // it disabled and retry, which is safer than deleting a config they just
        // typed. The service records the failed start as durable drift (AC-9).
        post_idempotent(
            supervisor,
            MCP_ENABLE_ROUTE,
            &json!({ "id": id }),
            &new_key(),
        )
        .await?;
    }
    mcp_get_record(supervisor, id).await
}

/// Updates one MCP server from the desktop form's whole configuration.
pub async fn mcp_update_server(
    supervisor: &RuntimeSupervisor,
    id: i64,
    name: &str,
    description: &str,
    config: &McpServerConfig,
    disabled: bool,
) -> CapResult<Mcp> {
    let body = json!({
        "id": id,
        "name": name,
        "description": description,
        "config": config,
        "disabled": disabled,
    });
    post_idempotent(supervisor, MCP_UPDATE_ROUTE, &body, &new_key()).await?;
    mcp_get_record(supervisor, id).await
}

/// Deletes one MCP server (disable, confirm the stop, then remove).
pub async fn mcp_delete_server(supervisor: &RuntimeSupervisor, id: i64) -> CapResult<()> {
    post_idempotent(
        supervisor,
        MCP_UNINSTALL_ROUTE,
        &json!({ "id": id }),
        &new_key(),
    )
    .await
    .map(|_| ())
}

/// Connects one MCP server (desired enabled plus a bounded start).
pub async fn mcp_enable_server(supervisor: &RuntimeSupervisor, id: i64) -> CapResult<()> {
    post_idempotent(
        supervisor,
        MCP_ENABLE_ROUTE,
        &json!({ "id": id }),
        &new_key(),
    )
    .await
    .map(|_| ())
}

/// Disconnects one MCP server (desired disabled plus a confirmed stop).
pub async fn mcp_disable_server(supervisor: &RuntimeSupervisor, id: i64) -> CapResult<()> {
    post_idempotent(
        supervisor,
        MCP_DISABLE_ROUTE,
        &json!({ "id": id }),
        &new_key(),
    )
    .await
    .map(|_| ())
}

/// Restarts one MCP server as one stop-then-start operation.
pub async fn mcp_restart_server(supervisor: &RuntimeSupervisor, id: i64) -> CapResult<()> {
    post_idempotent(
        supervisor,
        MCP_RESTART_ROUTE,
        &json!({ "id": id }),
        &new_key(),
    )
    .await
    .map(|_| ())
}

/// Re-lists one MCP server's tools without invoking any.
pub async fn mcp_refresh_server(supervisor: &RuntimeSupervisor, id: i64) -> CapResult<()> {
    post_idempotent(
        supervisor,
        MCP_REFRESH_ROUTE,
        &json!({ "id": id }),
        &new_key(),
    )
    .await
    .map(|_| ())
}

/// The typed cached tool declarations of one MCP server.
pub async fn mcp_server_tools(
    supervisor: &RuntimeSupervisor,
    id: i64,
) -> CapResult<Vec<MCPToolDeclaration>> {
    let path = format!("{MCP_TOOL_DECLARATIONS_ROUTE}?id={id}");
    decode(get_value(supervisor, &path).await?)
}

/// Enables or disables one cached MCP tool.
pub async fn mcp_update_tool_status(
    supervisor: &RuntimeSupervisor,
    id: i64,
    tool_name: &str,
    disabled: bool,
) -> CapResult<Mcp> {
    let body = json!({ "id": id, "tool_name": tool_name, "disabled": disabled });
    post_idempotent(supervisor, MCP_TOOL_STATUS_ROUTE, &body, &new_key()).await?;
    mcp_get_record(supervisor, id).await
}

// ---------------------------------------------------------------------------
// Client WebView capability bridge (U-7)
// ---------------------------------------------------------------------------

/// Legacy compatibility read for clients that still display the old capability
/// inventory. Runtime workflow execution does not use this route; it uses the
/// canonical MCP registry.
pub async fn client_capabilities(supervisor: &RuntimeSupervisor) -> CapResult<Value> {
    get_value(supervisor, CLIENT_CAPABILITIES_ROUTE).await
}

#[cfg(test)]
/// Legacy typed bridge invocation retained only for protocol regression tests.
pub async fn invoke_client_capability(
    supervisor: &RuntimeSupervisor,
    capability: &str,
    request: &Value,
) -> CapResult<Value> {
    supervisor
        .invoke_client_capability(capability, request)
        .await
        .map_err(map_runtime_error)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Builds the strict install descriptor from the desktop form's configuration.
///
/// `McpServerConfig` serializes to exactly the descriptor's accepted fields
/// (`type` for the transport, plus snake_case command/args/env/...), so the
/// descriptor only needs the identity fields layered on top.
fn install_descriptor(name: &str, description: &str, config: &McpServerConfig) -> CapResult<Value> {
    let mut descriptor = serde_json::to_value(config).map_err(serialization_error)?;
    let object = descriptor.as_object_mut().ok_or_else(|| {
        CapabilityError::new(
            code::INTERNAL,
            "the MCP configuration did not serialize to an object",
        )
    })?;
    object.insert("name".to_string(), Value::String(name.to_string()));
    object.insert(
        "description".to_string(),
        Value::String(description.to_string()),
    );
    Ok(descriptor)
}

/// Resolves the connected control-plane client, or fails when no lease is held.
async fn control_plane_client(supervisor: &RuntimeSupervisor) -> CapResult<RuntimeClient> {
    supervisor.client().await.map_err(map_runtime_error)
}

/// Performs a `GET` and returns the decoded JSON body.
async fn get_value(supervisor: &RuntimeSupervisor, path: &str) -> CapResult<Value> {
    let client = control_plane_client(supervisor).await?;
    client.get(path).await.map_err(map_client_error)
}

/// Performs an idempotent `POST` and returns the decoded JSON body.
async fn post_idempotent(
    supervisor: &RuntimeSupervisor,
    path: &str,
    body: &Value,
    idempotency_key: &str,
) -> CapResult<Value> {
    let client = control_plane_client(supervisor).await?;
    client
        .post_with_idempotency(path, body, idempotency_key)
        .await
        .map_err(map_client_error)
}

/// Decodes a runtime response into the command's expected type.
fn decode<T: DeserializeOwned>(value: Value) -> CapResult<T> {
    serde_json::from_value(value).map_err(|error| {
        CapabilityError::new(
            code::INTERNAL,
            format!("Unexpected runtime capability response: {error}"),
        )
    })
}

/// Reads the stored id a mutation result reported.
fn mutated_id(result: &Value) -> CapResult<i64> {
    result.get("id").and_then(Value::as_i64).ok_or_else(|| {
        CapabilityError::new(
            code::INTERNAL,
            "the runtime mutation result lost the record id",
        )
    })
}

/// Per-command idempotency key.
///
/// The legacy MCP wire has no key parameter and each invocation is one
/// deliberate user action, so a fresh key is the faithful mapping: the durable
/// operation still records what happened while the service's per-resource lock
/// and in-state short-circuits keep a double click from producing a second
/// effect.
fn new_key() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Maps a transport/domain client error into a capability error, preserving the
/// runtime's own stable code and already-redacted message.
fn map_client_error(error: ClientError) -> CapabilityError {
    match error {
        ClientError::Server {
            code: server_code,
            message,
            ..
        } => CapabilityError::new(&server_code, message),
        ClientError::Auth(message) => CapabilityError::new(code::FORBIDDEN, message),
        ClientError::InvalidRequest(message) => {
            CapabilityError::new(code::INVALID_REQUEST, message)
        }
        ClientError::Serialization(message) => CapabilityError::new(code::INTERNAL, message),
        other => CapabilityError::new(code::RUNTIME_UNAVAILABLE, other.to_string()),
    }
}

/// Maps a supervisor availability failure to `runtime_unavailable`, so the Tauri
/// wire fails closed instead of pretending the operation succeeded.
fn map_runtime_error(error: RuntimeUnavailable) -> CapabilityError {
    CapabilityError::new(code::RUNTIME_UNAVAILABLE, error.to_string())
}

/// Maps a request serialization failure to an internal capability error.
fn serialization_error(error: serde_json::Error) -> CapabilityError {
    CapabilityError::new(
        code::INTERNAL,
        format!("could not serialize the capability request: {error}"),
    )
}

/// Percent-encodes one path segment so a session/operation id cannot inject a
/// separator or a query.
fn encode_path_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        let unreserved = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~');
        if unreserved {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::client::McpProtocolType;

    fn stdio_config() -> McpServerConfig {
        McpServerConfig {
            name: "stale".to_string(),
            protocol_type: McpProtocolType::Stdio,
            command: Some("node".to_string()),
            args: Some(vec!["server.js".to_string()]),
            ..Default::default()
        }
    }

    #[test]
    fn the_install_descriptor_reuses_the_config_wire_and_overrides_identity() {
        let descriptor =
            install_descriptor("weather", "Weather data", &stdio_config()).expect("descriptor");

        // Identity comes from the command arguments, matching the legacy
        // `config.name = name` normalisation.
        assert_eq!(descriptor["name"], "weather");
        assert_eq!(descriptor["description"], "Weather data");
        // The transport and command fields keep their exact config wire keys.
        assert_eq!(descriptor["type"], "stdio");
        assert_eq!(descriptor["command"], "node");
        assert_eq!(descriptor["args"], json!(["server.js"]));
    }

    #[test]
    fn a_server_error_keeps_its_capability_code_and_message() {
        let error = map_client_error(ClientError::Server {
            status: 404,
            code: "not_found".to_string(),
            message: "MCP server 1 does not exist".to_string(),
        });
        assert_eq!(error.code(), code::NOT_FOUND);
        assert_eq!(error.message, "MCP server 1 does not exist");
    }

    #[test]
    fn a_transport_failure_maps_to_runtime_unavailable() {
        let error = map_client_error(ClientError::Transport("connection refused".to_string()));
        assert_eq!(error.code(), code::RUNTIME_UNAVAILABLE);
    }

    #[test]
    fn a_missing_lease_maps_to_runtime_unavailable() {
        let error = map_runtime_error(RuntimeUnavailable::NotConnected);
        assert_eq!(error.code(), code::RUNTIME_UNAVAILABLE);
    }

    #[test]
    fn path_segments_are_percent_encoded() {
        assert_eq!(encode_path_segment("op-1_a.b~c"), "op-1_a.b~c");
        assert_eq!(encode_path_segment("a/b"), "a%2Fb");
        assert_eq!(encode_path_segment("a?b=c"), "a%3Fb%3Dc");
    }
}
