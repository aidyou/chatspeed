//
//! This module contains Tauri commands for managing MCP (Model Context Protocol) servers.
//! It provides functionalities to list, add, update, delete, and connect/disconnect MCP servers,
//! as well as to retrieve the tools provided by each server.
//!
//! ## Overview
//!
//! - **MCP Servers**: Functions to manage MCP servers, including adding, updating,
//!   deleting, and retrieving servers.
//! - **Connection Management**: Functions to connect and disconnect from MCP servers.
//! - **Tools**: Functions to retrieve the tools provided by each MCP server.
//!
//! ## Usage
//!
//! The commands can be invoked from the frontend using Tauri's `invoke` function.
//! Each command is annotated with detailed documentation, including parameters,
//! return types, and examples of usage.
//!

use crate::{
    ai::{interaction::chat_completion::ChatState, traits::chat::MCPToolDeclaration},
    db::{MainStore, Mcp},
    error::{AppError, Result},
    mcp::client::{McpProtocolType, McpServerConfig, McpStatus},
    mcp::McpError,
};
use rust_i18n::t;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::State;

use crate::capability::error::code;
use crate::capability::error::CapabilityError;
use crate::capability::mcp_service::public_runtime_status;
use crate::capability::CapabilityApplicationService;

/// Get all MCP servers
///
/// Retrieves a list of all MCP servers from the database.
///
/// # Arguments
/// - `main_store` - The state of the main application store, automatically injected by Tauri.
/// - `chat_state` - The state of the chat system, automatically injected by Tauri.
///
/// # Returns
/// * `Result<Vec<Mcp>, String>` - A vector of MCP servers with their current status, or an error message.
///   The `config` of each record is secret-free: the bearer token and every
///   environment value are removed by the shared capability projection, so the
///   page can edit the non-sensitive fields without a credential crossing the
///   IPC boundary (AC-13). Presence is reported by `capability_mcp_servers`.
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core'
///
/// const servers = await invoke('list_mcp_servers');
/// console.log(servers);
/// ```
#[tauri::command]
pub async fn list_mcp_servers(
    capability: State<'_, Arc<CapabilityApplicationService>>,
    chat_state: State<'_, Arc<ChatState>>,
) -> Result<Vec<Mcp>> {
    // The single capability service owns the records; it hands back the legacy
    // editable `Mcp` shape with secrets already stripped (INV-1/AC-13). The old
    // command name and non-sensitive wire fields are unchanged for the page.
    let mut mcps = capability
        .mcp_records_redacted()
        .await
        .map_err(legacy_error)?;

    // Get the status of each MCP server and update the status field. A live
    // error status is projected through `public_runtime_status` so a runtime
    // message that interpolates a token / env value / URL credential can never
    // cross the IPC boundary (AC-13).
    if let Ok(status_map) = chat_state
        .tool_manager
        .clone()
        .get_mcp_serves_status()
        .await
    {
        overlay_redacted_status(&mut mcps, &status_map);
    }
    Ok(mcps)
}

/// Overlays the live runtime status onto the already-redacted records, routing
/// every status through [`public_runtime_status`].
///
/// This is the exact transform `list_mcp_servers` applies to build the desktop
/// response, so the secret-canary regression test can exercise it directly
/// without a live Tauri runtime (AC-13).
fn overlay_redacted_status(mcps: &mut [Mcp], status_map: &HashMap<String, McpStatus>) {
    for mcp in mcps.iter_mut() {
        if let Some(status) = status_map.get(&mcp.name) {
            mcp.status = Some(public_runtime_status(status));
        }
    }
}

/// check the form of the MCP server config
///
/// # Arguments
/// - `name` - The name of the MCP server.
/// - `config` - The configuration of the MCP server.
///
/// # Returns
/// * `Result<(), String>` - An error message if the form is invalid, or `Ok(())` if the form is valid.
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

/// Adds a new MCP server to the database.
///
/// # Arguments
/// - `main_store` - The state of the main application store.
/// - `chat_state` - The state of the chat system.
/// - `name` - The name of the new MCP server.
/// - `description` - A description for the new MCP server.
/// - `config` - The `McpServerConfig` for the new server.
/// - `disabled` - A boolean indicating whether the server should be initially disabled.
///
/// # Returns
/// * `Result<Mcp, String>` - The added MCP server data or an error message.
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core'
///
/// const server = await invoke('add_mcp_server', {
///     name: 'weather-server',
///     description: 'Provides weather information',
///     config: { // McpServerConfig object
///         name: 'weather-server', // Ensure this name matches the outer name
///         description: 'Weather data server',
///         config: {
///             type: 'stdio',
///             command: 'node',
///             args: ['weather-server.js'],
///             env: [['API_KEY', '12345']]
///         },
///     },
///     disabled: false
/// });
/// console.log('Added MCP server:', server);
/// ```
/// The journal actor scope for mutations started from the desktop window.
const ACTOR_DESKTOP: &str = "desktop";

/// One idempotency key per user action.
///
/// The legacy MCP wire has no key parameter and each invocation of these
/// commands *is* one deliberate user action, so a fresh key is the faithful
/// mapping: the durable operation still records what happened, while the
/// service's per-resource lock and its already-in-state short-circuits keep a
/// double click from producing a second process.
fn fresh_key(action: &str) -> String {
    format!("tauri-{action}-{}", uuid::Uuid::now_v7())
}

/// Maps a capability failure onto the `McpError` shapes the page already handles.
fn legacy_error(error: CapabilityError) -> AppError {
    let message = error.redacted_message();
    match error.code() {
        code::NOT_FOUND | code::OPERATION_NOT_FOUND => {
            AppError::Mcp(McpError::NotFound(message))
        }
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

/// Reads back the stored record for a command whose wire returns `Mcp`.
///
/// The returned record is secret-redacted, so `add`/`update`/`update_tool_status`
/// responses never echo a token or env value back over IPC (AC-13).
async fn stored_record(capability: &CapabilityApplicationService, id: i64) -> Result<Mcp> {
    capability
        .mcp_record_redacted(id)
        .await
        .map_err(legacy_error)?
        .ok_or_else(|| AppError::Mcp(McpError::NotFound("the MCP record vanished".into())))
}

/// The stored id a mutation reported, for the commands that must return `Mcp`.
fn mutated_id(result: &serde_json::Value) -> Result<i64> {
    result
        .get("id")
        .and_then(|value| value.as_i64())
        .ok_or_else(|| AppError::Mcp(McpError::General("capability result lost the id".into())))
}


#[tauri::command]
pub async fn add_mcp_server(
    capability: State<'_, Arc<CapabilityApplicationService>>,
    name: String,
    description: String,
    mut config: McpServerConfig,
    disabled: bool,
) -> Result<Mcp> {
    check_form(&name, &config)?;
    // The stored name is authoritative, exactly as before.
    config.name = name.clone();

    // One durable operation registers the server, always disabled. Starting it is
    // a second, separately auditable effect (AC-9), so a server that persists but
    // fails to start is visible as drift rather than a half-added record.
    let installed = capability
        .mcp_install_config(
            &name,
            &description,
            config,
            &fresh_key("install"),
            ACTOR_DESKTOP,
        )
        .await
        .map_err(legacy_error)?;
    let id = mutated_id(&installed.result)?;

    if !disabled {
        if let Err(error) = capability
            .mcp_enable(id, &fresh_key("enable"), ACTOR_DESKTOP)
            .await
        {
            // The record is intentionally kept: the user can see it disabled and
            // retry, which is safer than deleting a config they just typed.
            return Err(legacy_error(error));
        }
    }

    stored_record(&capability, id).await
}

/// Update an existing MCP server
///
/// Updates the configuration of an existing MCP server in the database.
///
/// # Arguments
/// - `main_store` - The state of the main application store.
/// - `chat_state` - The state of the chat system.
/// - `id` - The ID of the MCP server to update.
/// - `name` - The new name for the MCP server.
/// - `description` - The new description for the MCP server.
/// - `config` - The new `McpServerConfig`.
/// - `disabled` - The new disabled status.
/// - `disabled_tools` - An optional list of tool names to disable for this server.
///
/// # Returns
/// * `Result<Mcp, String>` - The updated MCP server data or an error message.
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core'
///
/// const server = await invoke('update_mcp_server', {
///     id: 1,
///     name: 'weather-server-updated',
///     description: 'Updated weather data server',
///     config: { // McpServerConfig object
///         name: 'weather-server-updated', // Ensure this name matches the outer name
///         description: 'Updated weather data server', // This field in McpServerConfig might be redundant if also top-level
///         config: {
///             type: 'stdio',
///             command: 'node',
///             args: ['updated-server.js'],
///             env: [['API_KEY', '67890']]
///         },
///         disabled: false
///     },
///     disabled_tools: ['old_tool']
/// });
/// console.log('Updated MCP server:', server);
/// ```

#[tauri::command]
pub async fn update_mcp_server(
    capability: State<'_, Arc<CapabilityApplicationService>>,
    id: i64,
    name: String,
    description: String,
    mut config: McpServerConfig,
    disabled: bool,
) -> Result<Mcp> {
    check_form(&name, &config)?;
    config.name = name.clone();

    capability
        .mcp_update(
            id,
            &name,
            &description,
            config,
            disabled,
            &fresh_key("update"),
            ACTOR_DESKTOP,
        )
        .await
        .map_err(legacy_error)?;

    stored_record(&capability, id).await
}

/// Delete an MCP server
///
/// Removes an MCP server from the database by its ID.
///
/// # Arguments
/// - `main_store` - The state of the main application store.
/// - `chat_state` - The state of the chat system.
/// - `id` - The ID of the MCP server to delete.
///
/// # Returns
/// * `Result<(), String>` - Ok if successful, or an error message.
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core'
///
/// await invoke('delete_mcp_server', { name: 'weather-server' });
/// await invoke('delete_mcp_server', { id: 1 });
/// ```

#[tauri::command]
pub async fn delete_mcp_server(
    capability: State<'_, Arc<CapabilityApplicationService>>,
    id: i64,
) -> Result<()> {
    // Uninstall is ordered desired-disabled, confirmed stop, then delete. When the
    // stop cannot be confirmed the record is kept and the operation asks for
    // reconciliation, so no child process is ever orphaned by a successful-looking
    // delete (AC-10).
    capability
        .mcp_uninstall(id, &fresh_key("uninstall"), ACTOR_DESKTOP)
        .await
        .map_err(legacy_error)?;
    Ok(())
}

/// Connect to an MCP server
/// Establishes a connection to the specified MCP server.
///
/// # Arguments
/// - `main_store` - The state of the main application store.
/// - `chat_state` - The state of the chat system.
/// - `id` - The ID of the MCP server to connect to.
///
/// # Returns
/// * `Result<(), String>` - Ok if successful, or an error message.
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core'
///
/// await invoke('enable_mcp_server', { id: 1 });
/// ```

#[tauri::command]
pub async fn enable_mcp_server(
    capability: State<'_, Arc<CapabilityApplicationService>>,
    id: i64,
) -> Result<()> {
    // Desired state and runtime observation are one durable operation with a
    // bounded wait: the caller is told the truth about whether the server is
    // actually up, instead of an untracked background task deciding later
    // (INV-7).
    capability
        .mcp_enable(id, &fresh_key("enable"), ACTOR_DESKTOP)
        .await
        .map_err(legacy_error)?;
    Ok(())
}

/// Disconnect from an MCP server
/// Closes the connection to the specified MCP server.
///
/// # Arguments
/// - `main_store` - The state of the main application store.
/// - `chat_state` - The state of the chat system.
/// - `id` - The ID of the MCP server to disconnect from.
///
/// # Returns
/// * `Result<(), String>` - Ok if successful, or an error message.
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core'
///
/// await invoke('disabled_mcp_server', { id: 1 });
/// ```

#[tauri::command]
pub async fn disable_mcp_server(
    capability: State<'_, Arc<CapabilityApplicationService>>,
    id: i64,
) -> Result<()> {
    // The stop is confirmed by observation before the command reports success. An
    // unconfirmed stop keeps the disabled record and asks for reconciliation, so a
    // later uninstall cannot be told "gone" while a child is still alive (AC-10).
    capability
        .mcp_disable(id, &fresh_key("disable"), ACTOR_DESKTOP)
        .await
        .map_err(legacy_error)?;
    Ok(())
}


#[tauri::command]
pub async fn restart_mcp_server(
    capability: State<'_, Arc<CapabilityApplicationService>>,
    id: i64,
) -> Result<()> {
    let error = match capability
        .mcp_restart(id, &fresh_key("restart"), ACTOR_DESKTOP)
        .await
    {
        Ok(_) => return Ok(()),
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
/// This command fetches the latest tool list from the specified MCP server
/// and updates the application's in-memory state.
///
/// # Arguments
/// - `chat_state` - The state of the chat system.
/// - `main_store` - The state of the main application store.
/// - `id` - The ID of the MCP server to refresh.
///
/// # Returns
/// * `Result<(), String>` - Ok if successful, or an error message.

#[tauri::command]
pub async fn refresh_mcp_server(
    capability: State<'_, Arc<CapabilityApplicationService>>,
    id: i64,
) -> Result<()> {
    let error = match capability
        .mcp_refresh_tools(id, &fresh_key("refresh"), ACTOR_DESKTOP)
        .await
    {
        Ok(_) => return Ok(()),
        Err(error) => error,
    };
    if error.code() == code::REFUSED {
        return Err(AppError::Mcp(McpError::StateChangeFailed(
            t!("mcp.error.cannot_refresh_disabled_server").to_string(),
        )));
    }
    Err(legacy_error(error))
}

/// Get tools from an MCP server
///
/// Retrieves the list of tools provided by the specified MCP server.
///
/// # Arguments
/// - `main_store` - The state of the main application store.
/// - `chat_state` - The state of the chat system.
/// - `id` - The ID of the MCP server to get tools from.
///
/// # Returns
/// * `Result<Vec<MCPToolDeclaration>, String>` - A vector of tool declarations or an error message.
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core'
///
/// const tools = await invoke('get_mcp_server_tools', { id: 1 });
/// console.log('MCP server tools:', tools);
/// ```

#[tauri::command]
pub async fn get_mcp_server_tools(
    capability: State<'_, Arc<CapabilityApplicationService>>,
    id: i64,
) -> Result<Vec<MCPToolDeclaration>> {
    // Listing is a read of what the runtime already holds. It never starts a
    // server and never invokes a tool (AC-11).
    capability
        .mcp_tool_declarations(id)
        .await
        .map_err(|error| AppError::Mcp(McpError::NotFound(error.redacted_message())))
}


#[tauri::command]
pub async fn update_mcp_tool_status(
    capability: State<'_, Arc<CapabilityApplicationService>>,
    id: i64,
    tool_name: String,
    disabled: bool,
) -> Result<Mcp> {
    capability
        .mcp_set_tool_disabled(
            id,
            &tool_name,
            disabled,
            &fresh_key("tool-state"),
            ACTOR_DESKTOP,
        )
        .await
        .map_err(legacy_error)?;
    stored_record(&capability, id).await
}

/// Invoke an MCP tool with the given arguments (manual execution / testing).
///
/// # Arguments
/// - `main_store` - The state of the main application store, automatically injected by Tauri.
/// - `chat_state` - The state of the chat system, automatically injected by Tauri.
/// - `id` - The ID of the MCP server.
/// - `tool_name` - The name of the tool to invoke on the MCP server.
/// - `arguments` - A JSON object of arguments to pass to the tool.
///
/// # Returns
/// * `Result<serde_json::Value, String>` - The raw result returned by the MCP server,
///   or an error message.
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core'
///
/// const result = await invoke('run_mcp_tool', {
///   id: 1,
///   toolName: 'read_file',
///   arguments: { path: '/tmp/test.txt' }
/// });
/// console.log('MCP tool result:', result);
/// ```
#[tauri::command]
pub async fn run_mcp_tool(
    main_store: State<'_, Arc<MainStore>>,
    chat_state: State<'_, Arc<ChatState>>,
    id: i64,
    tool_name: &str,
    arguments: serde_json::Value,
) -> Result<serde_json::Value> {
    let mcp_name = {
        let store_guard = &*main_store;
        let mcp = store_guard.config.get_mcp_by_id(id)?;
        mcp.name.clone()
    };

    let tool_manager = chat_state.tool_manager.clone();

    // The server must be registered (running) to be callable.
    let client = tool_manager
        .get_mcp_server(&mcp_name)
        .await
        .map_err(|e| AppError::Mcp(McpError::NotFound(e.to_string())))?;

    // Verify the tool exists and is not disabled before invoking it.
    let tools = tool_manager
        .get_mcp_server_tools(&mcp_name)
        .await
        .map_err(|e| AppError::Mcp(McpError::NotFound(e.to_string())))?;
    let declaration = tools
        .iter()
        .find(|declaration| declaration.name == tool_name)
        .ok_or_else(|| AppError::Mcp(McpError::ServerToolNotFound(tool_name.to_string())))?;
    if declaration.disabled {
        return Err(AppError::Mcp(McpError::General(format!(
            "MCP tool '{}' on server '{}' is disabled",
            tool_name, mcp_name
        ))));
    }

    let result = client
        .call(tool_name, arguments)
        .await
        .map_err(AppError::Mcp)?;

    Ok(result)
}

#[cfg(test)]
mod characterization {
    //! These tests pin the *existing* MCP command contract before the command
    //! bodies are migrated onto the shared capability service (U-8 -> U-9). They
    //! are characterization, not aspiration: if a migration changes any of these
    //! shapes, the legacy desktop wire compatibility guarantee (INV-2) is broken
    //! and these tests must fail.

    use super::*;
    use crate::capability::mcp_service::redact_record_secrets;
    use crate::mcp::client::{McpProtocolType, McpStatus};
    use serde_json::Value;

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
        let error = check_form(
            "weather",
            &config_with(McpProtocolType::Sse, None, None),
        )
        .expect_err("sse must be refused");
        assert!(matches!(error, AppError::Mcp(McpError::ClientConfigError(_))));
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
        assert!(config.get("bearer_token").is_none(), "absent secrets stay absent");
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
        // raw `McpStatus` enum itself keeps the full error message, which the
        // MCP client builds from config values. The capability projection (state
        // name only) and `list_mcp_servers` (via `public_runtime_status`) are the
        // secret-free surfaces; the raw type is never handed to the page directly
        // (INV-7/AC-13).
        let status = McpStatus::Error("handshake failed with token=canary".to_string());
        let value = serde_json::to_value(&status).expect("serialize status");
        assert_eq!(value["error"], "handshake failed with token=canary");
    }

    #[test]
    fn the_desktop_list_response_carries_no_secret_in_the_overlaid_status() {
        // Exercises the exact `list_mcp_servers` transform: redact the stored
        // config, then overlay a live runtime error whose message embeds a URL
        // credential, an inline token and a bare bearer value. The serialized
        // `Vec<Mcp>` the command actually returns to the desktop must contain
        // none of them (AC-13).
        let stored = Mcp {
            id: 1,
            name: "weather".to_string(),
            description: String::new(),
            config: McpServerConfig {
                bearer_token: Some("CONFIG-BEARER-CANARY".to_string()),
                env: Some(vec![(
                    "API_TOKEN".to_string(),
                    "CONFIG-ENV-CANARY".to_string(),
                )]),
                ..stdio_config()
            },
            disabled: false,
            status: None,
        };
        let mut records = vec![redact_record_secrets(&stored)];

        let mut status_map = HashMap::new();
        status_map.insert(
            "weather".to_string(),
            McpStatus::Error(
                "connect https://user:URL-USERINFO-CANARY@host.test failed token=INLINE-TOKEN-CANARY raw-BEARER-CANARY"
                    .to_string(),
            ),
        );
        overlay_redacted_status(&mut records, &status_map);

        let serialized = serde_json::to_string(&records).expect("serialize desktop response");
        for canary in [
            "CONFIG-BEARER-CANARY",
            "CONFIG-ENV-CANARY",
            "URL-USERINFO-CANARY",
            "INLINE-TOKEN-CANARY",
            "raw-BEARER-CANARY",
        ] {
            assert!(
                !serialized.contains(canary),
                "{canary} leaked into the desktop list response: {serialized}"
            );
        }
        // The page still sees an error status with its object wire shape (INV-2).
        assert!(matches!(
            records[0].status,
            Some(McpStatus::Error(_))
        ));
        // The editable non-sensitive config survives the redaction.
        assert_eq!(records[0].config.command.as_deref(), Some("node"));
    }
}
