//! Tauri commands for managing MCP (Model Context Protocol) servers.
//!
//! Every command here is a thin transport over the standalone runtime control
//! plane. The runtime process owns the MCP records, the capability journal and
//! the MCP runtime itself, so these commands reach it through the typed
//! [`crate::runtime_client::RuntimeSupervisor`] and the explicit
//! `/control/v1` MCP routes; they never touch `MainStore`, `ChatState` or a
//! `ToolManager`, and there is no local fallback (INV-1/INV-7).
//!
//! The legacy Tauri wire is preserved: command names, parameters, the
//! secret-free editable `Mcp` responses and the error classification are all
//! unchanged, and the live status now comes from the runtime's own observation
//! rather than a client-local tool manager.
//!
//! `run_mcp_tool` is the one command that is deliberately non-durable: a manual
//! invocation reaches the runtime owner through the fixed `/control/v1/mcp-call`
//! route, opens no journal and carries no idempotency key, and the exact MCP
//! result is returned unchanged. The runtime validates the record, the runtime
//! state, the tool ownership and the disabled flag before any call reaches the
//! MCP server (AC-11).

use crate::ai::traits::chat::MCPToolDeclaration;
use crate::capability::error::{code, CapabilityError};
use crate::commands::capability::runtime_capability;
use crate::db::Mcp;
use crate::error::{AppError, Result};
use crate::mcp::client::{McpProtocolType, McpServerConfig};
use crate::mcp::McpError;
use crate::runtime_client::RuntimeSupervisor;
use rust_i18n::t;
use serde_json::Value;
use std::sync::Arc;
use tauri::State;

/// Get all MCP servers.
///
/// Returns the secret-free editable `Mcp` records (`command`/`args`/`url`/
/// `proxy`/`timeout`/`disabled_tools`) with the live runtime status already
/// overlaid by the runtime, so the page can edit the non-sensitive fields
/// without a credential crossing the IPC boundary and without a second status
/// source (AC-13/INV-7).
///
/// # Example
///
/// ```js
/// const servers = await invoke('list_mcp_servers');
/// ```
#[tauri::command]
pub async fn list_mcp_servers(supervisor: State<'_, Arc<RuntimeSupervisor>>) -> Result<Vec<Mcp>> {
    runtime_capability::mcp_list_records(supervisor.inner().as_ref())
        .await
        .map_err(legacy_error)
}

/// check the form of the MCP server config
///
/// The desktop form is still validated before the round trip so an obvious
/// mistake (empty name, removed `sse` transport, stdio without command/args) is
/// reported without a failure the runtime would have to journal.
fn check_form(name: &str, config: &McpServerConfig) -> Result<()> {
    if name.is_empty() || config.name.is_empty() {
        return Err(AppError::Mcp(McpError::ClientConfigError(
            t!("mcp.config.name_must_be_non_empty").to_string(),
        )));
    }

    if config.protocol_type == McpProtocolType::Sse {
        return Err(AppError::Mcp(McpError::ClientConfigError(
            t!("mcp.config.sse_removed_in_rmcp_v1").to_string(),
        )));
    }

    if config.protocol_type == McpProtocolType::Stdio {
        if config.command.clone().unwrap_or_default().is_empty() {
            return Err(AppError::Mcp(McpError::ClientConfigError(
                t!("mcp.config.stdio_command_must_be_non_empty").to_string(),
            )));
        }
        if config.args.clone().unwrap_or_default().is_empty() {
            return Err(AppError::Mcp(McpError::ClientConfigError(
                t!("mcp.config.stdio_args_must_be_non_empty").to_string(),
            )));
        }
    }

    Ok(())
}

/// Maps a capability failure onto the `McpError` shapes the page already handles.
fn legacy_error(error: CapabilityError) -> AppError {
    let message = error.redacted_message();
    match error.code() {
        code::NOT_FOUND | code::OPERATION_NOT_FOUND => AppError::Mcp(McpError::NotFound(message)),
        code::INVALID_REQUEST | code::UNSUPPORTED_ADAPTER | code::IDEMPOTENCY_KEY_REQUIRED => {
            AppError::Mcp(McpError::ClientConfigError(message))
        }
        code::BUSY
        | code::REFUSED
        | code::NEEDS_RECONCILE
        | code::EFFECT_STATE_UNKNOWN
        | code::RUNTIME_UNAVAILABLE
        | code::INTERRUPTED_BEFORE_EFFECT
        | code::PARTIAL => AppError::Mcp(McpError::StateChangeFailed(message)),
        _ => AppError::Mcp(McpError::General(message)),
    }
}

/// Adds a new MCP server to the runtime, always disabled, then optionally
/// connects it.
///
/// One durable operation registers the server; starting it is a second,
/// separately auditable effect (AC-9), so a server that persists but fails to
/// start is visible as drift rather than a half-added record. The record is
/// intentionally kept when the start fails, so the user can see it disabled and
/// retry.
///
/// # Example
///
/// ```js
/// const server = await invoke('add_mcp_server', {
///   name: 'weather-server',
///   description: 'Provides weather information',
///   config: { name: 'weather-server', type: 'stdio', command: 'node', args: ['weather-server.js'] },
///   disabled: false,
/// });
/// ```
#[tauri::command]
pub async fn add_mcp_server(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    name: String,
    description: String,
    mut config: McpServerConfig,
    disabled: bool,
) -> Result<Mcp> {
    check_form(&name, &config)?;
    // The stored name is authoritative, exactly as before.
    config.name = name.clone();

    runtime_capability::mcp_add_server(
        supervisor.inner().as_ref(),
        &name,
        &description,
        &config,
        disabled,
    )
    .await
    .map_err(legacy_error)
}

/// Update an existing MCP server.
///
/// # Example
///
/// ```js
/// const server = await invoke('update_mcp_server', { id: 1, name, description, config, disabled: false });
/// ```
#[tauri::command]
pub async fn update_mcp_server(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
    name: String,
    description: String,
    mut config: McpServerConfig,
    disabled: bool,
) -> Result<Mcp> {
    check_form(&name, &config)?;
    config.name = name.clone();

    runtime_capability::mcp_update_server(
        supervisor.inner().as_ref(),
        id,
        &name,
        &description,
        &config,
        disabled,
    )
    .await
    .map_err(legacy_error)
}

/// Delete an MCP server.
///
/// Uninstall is ordered desired-disabled, confirmed stop, then delete. When the
/// stop cannot be confirmed the record is kept and the operation asks for
/// reconciliation, so no child process is ever orphaned by a successful-looking
/// delete (AC-10).
#[tauri::command]
pub async fn delete_mcp_server(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
) -> Result<()> {
    runtime_capability::mcp_delete_server(supervisor.inner().as_ref(), id)
        .await
        .map_err(legacy_error)
}

/// Connect to an MCP server.
///
/// Desired state and runtime observation are one durable operation with a
/// bounded wait: the caller is told the truth about whether the server is
/// actually up, instead of an untracked background task deciding later (INV-7).
#[tauri::command]
pub async fn enable_mcp_server(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
) -> Result<()> {
    runtime_capability::mcp_enable_server(supervisor.inner().as_ref(), id)
        .await
        .map_err(legacy_error)
}

/// Disconnect from an MCP server.
///
/// The stop is confirmed by observation before the command reports success. An
/// unconfirmed stop keeps the disabled record and asks for reconciliation, so a
/// later uninstall cannot be told "gone" while a child is still alive (AC-10).
#[tauri::command]
pub async fn disable_mcp_server(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
) -> Result<()> {
    runtime_capability::mcp_disable_server(supervisor.inner().as_ref(), id)
        .await
        .map_err(legacy_error)
}

/// Restart an MCP server.
#[tauri::command]
pub async fn restart_mcp_server(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
) -> Result<()> {
    let error = match runtime_capability::mcp_restart_server(supervisor.inner().as_ref(), id).await
    {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    // The page's existing copy for this case is a localized message, so the
    // adapter keeps wording it while the decision itself stays in the service.
    if error.code() == code::REFUSED {
        return Err(AppError::Mcp(McpError::StateChangeFailed(
            t!("mcp.error.cannot_restart_disabled_server").to_string(),
        )));
    }
    Err(legacy_error(error))
}

/// Refresh the tool list for an MCP server.
///
/// Listing never invokes a tool; it re-reads what the runtime already holds
/// (AC-11).
#[tauri::command]
pub async fn refresh_mcp_server(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
) -> Result<()> {
    let error = match runtime_capability::mcp_refresh_server(supervisor.inner().as_ref(), id).await
    {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    if error.code() == code::REFUSED {
        return Err(AppError::Mcp(McpError::StateChangeFailed(
            t!("mcp.error.cannot_refresh_disabled_server").to_string(),
        )));
    }
    Err(legacy_error(error))
}

/// Get tools from an MCP server.
///
/// Listing is a read of what the runtime already holds. It never starts a server
/// and never invokes a tool (AC-11).
#[tauri::command]
pub async fn get_mcp_server_tools(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
) -> Result<Vec<MCPToolDeclaration>> {
    runtime_capability::mcp_server_tools(supervisor.inner().as_ref(), id)
        .await
        .map_err(|error| AppError::Mcp(McpError::NotFound(error.redacted_message())))
}

/// Enable or disable one tool of an MCP server.
#[tauri::command]
pub async fn update_mcp_tool_status(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
    tool_name: String,
    disabled: bool,
) -> Result<Mcp> {
    runtime_capability::mcp_update_tool_status(
        supervisor.inner().as_ref(),
        id,
        &tool_name,
        disabled,
    )
    .await
    .map_err(legacy_error)
}

/// Manually invoke an MCP tool (manual execution / testing).
///
/// The invocation is delegated to the runtime owner through the fixed
/// `/control/v1/mcp-call` route, which validates the record, the runtime state,
/// the tool ownership and the disabled flag before any call reaches the MCP
/// server. The runtime's exact JSON result is returned unchanged.
///
/// A manual invocation is deliberately non-durable: it opens no journal and
/// carries no idempotency key, so a retry runs again instead of replaying.
#[tauri::command]
pub async fn run_mcp_tool(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
    tool_name: String,
    arguments: Value,
) -> Result<Value> {
    runtime_capability::mcp_call(supervisor.inner().as_ref(), id, &tool_name, &arguments)
        .await
        .map_err(legacy_error)
}

#[cfg(test)]
mod characterization {
    //! These tests pin the *existing* MCP command contract. They are
    //! characterization, not aspiration: if a migration changes any of these
    //! shapes, the legacy desktop wire compatibility guarantee (INV-2) is broken
    //! and these tests must fail.

    use super::*;
    use crate::capability::mcp_service::redact_record_secrets;
    use crate::mcp::client::{McpProtocolType, McpStatus};

    fn config_with(
        protocol_type: McpProtocolType,
        command: Option<&str>,
        args: Option<Vec<String>>,
    ) -> McpServerConfig {
        McpServerConfig {
            name: "weather".to_string(),
            protocol_type,
            url: None,
            bearer_token: None,
            proxy: None,
            command: command.map(str::to_string),
            args,
            env: None,
            disabled_tools: None,
            timeout: None,
        }
    }

    fn stdio_config() -> McpServerConfig {
        config_with(
            McpProtocolType::Stdio,
            Some("node"),
            Some(vec!["server.js".to_string()]),
        )
    }

    #[test]
    fn a_valid_stdio_config_passes_the_shared_form_gate() {
        check_form("weather", &stdio_config()).expect("valid stdio config");
    }

    #[test]
    fn sse_is_rejected_by_the_form_gate() {
        // SSE was removed in rmcp v1; add/update must keep refusing it so a
        // migrated command cannot quietly start accepting it.
        let error = check_form("weather", &config_with(McpProtocolType::Sse, None, None))
            .expect_err("sse must be refused");
        assert!(matches!(
            error,
            AppError::Mcp(McpError::ClientConfigError(_))
        ));
    }

    #[test]
    fn stdio_requires_both_command_and_args() {
        let missing_command = config_with(McpProtocolType::Stdio, None, Some(vec!["a".into()]));
        assert!(check_form("weather", &missing_command).is_err());

        let missing_args = config_with(McpProtocolType::Stdio, Some("node"), None);
        assert!(check_form("weather", &missing_args).is_err());

        let empty_args = config_with(McpProtocolType::Stdio, Some("node"), Some(vec![]));
        assert!(check_form("weather", &empty_args).is_err());
    }

    #[test]
    fn an_empty_outer_or_config_name_is_rejected() {
        assert!(check_form("", &stdio_config()).is_err());

        let mut config = stdio_config();
        config.name = String::new();
        assert!(check_form("weather", &config).is_err());
    }

    #[test]
    fn the_record_wire_shape_the_desktop_page_depends_on_is_stable() {
        let record = Mcp {
            id: 7,
            name: "weather".to_string(),
            description: "Weather data".to_string(),
            config: stdio_config(),
            disabled: false,
            status: Some(McpStatus::Running),
        };

        let value = serde_json::to_value(&record).expect("serialize record");
        // Top-level snake_case keys the existing `stores/mcp.js` reads.
        assert_eq!(value["id"], Value::from(7));
        assert_eq!(value["name"], "weather");
        assert_eq!(value["disabled"], Value::from(false));
        assert_eq!(value["status"], "running");

        // The nested config keeps its exact casing: `type` for the protocol and
        // snake_case secret fields. A migration that renames any of these would
        // break the page and the persisted JSON in the database.
        let config = &value["config"];
        assert_eq!(config["type"], "stdio");
        assert_eq!(config["command"], "node");
        assert!(
            config.get("bearer_token").is_none(),
            "absent secrets stay absent"
        );
        assert!(config.get("env").is_none());

        let with_secret = Mcp {
            id: 7,
            name: "weather".to_string(),
            description: "Weather data".to_string(),
            config: McpServerConfig {
                bearer_token: Some("legacy-token".to_string()),
                env: Some(vec![("API_TOKEN".to_string(), "legacy-value".to_string())]),
                ..stdio_config()
            },
            disabled: false,
            status: Some(McpStatus::Running),
        };
        let raw = serde_json::to_value(&with_secret).expect("serialize secret record");
        // The raw `Mcp` is only the add/update INPUT and the persisted shape; it
        // legitimately carries the secret fields so a typed config round-trips.
        assert_eq!(raw["config"]["bearer_token"], "legacy-token");
        assert_eq!(
            raw["config"]["env"],
            serde_json::json!([["API_TOKEN", "legacy-value"]])
        );

        // The public READ path (`list_mcp_servers`, and the `Mcp` returned by
        // add/update) must never carry those values (AC-13). This is the exact
        // shape the command serializes over Tauri IPC.
        let public = serde_json::to_value(redact_record_secrets(&with_secret))
            .expect("serialize redacted read model");
        let public_str = serde_json::to_string(&public).expect("stringify");
        assert!(
            public["config"].get("bearer_token").is_none(),
            "bearer token must be absent from the read model"
        );
        assert!(
            public["config"].get("env").is_none(),
            "env values must be absent from the read model"
        );
        assert!(
            !public_str.contains("legacy-token") && !public_str.contains("legacy-value"),
            "a secret value must never serialize into the desktop read: {public_str}"
        );
        // Non-sensitive config still round-trips so the page can edit it, and the
        // status/desired wire the page reads stays intact (INV-2).
        assert_eq!(public["config"]["command"], "node");
        assert_eq!(public["config"]["type"], "stdio");
        assert_eq!(public["status"], "running");
    }

    #[test]
    fn an_absent_status_serializes_as_null_the_page_checks_for() {
        let record = Mcp {
            id: 1,
            name: "weather".to_string(),
            description: String::new(),
            config: stdio_config(),
            disabled: true,
            status: None,
        };
        let value = serde_json::to_value(&record).expect("serialize");
        assert_eq!(value["status"], Value::Null);
        assert_eq!(value["disabled"], Value::from(true));
    }

    #[test]
    fn a_raw_mcp_status_type_still_serializes_its_message() {
        // This documents WHY the desktop read path must project the status: the
        // raw `McpStatus` enum itself keeps the full error message, which the MCP
        // client builds from config values. The runtime projection (state name
        // only) is the secret-free surface; the raw type is never handed to the
        // page directly (INV-7/AC-13).
        let status = McpStatus::Error("handshake failed with token=canary".to_string());
        let value = serde_json::to_value(&status).expect("serialize status");
        assert_eq!(value["error"], "handshake failed with token=canary");
    }
}
