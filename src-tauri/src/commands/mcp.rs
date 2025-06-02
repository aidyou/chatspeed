//!
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
    mcp::client::{McpProtocolType, McpServerConfig},
};
use rust_i18n::t;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
use tauri::State;

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
    main_store: State<'_, Arc<Mutex<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
) -> Result<Vec<Mcp>, String> {
    // First, get the MCPs from the main_store.
    let mut mcps = {
        let store_guard = main_store
            .lock()
            .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
        store_guard.config.get_mcps()
    }; // store_guard is dropped here, releasing the lock.

    // Get the status of each MCP server and update the status field
    // This part involves async operations, so the lock on main_store must be released.
    if let Ok(status_map) = chat_state
        .tool_manager
        .clone()
        .get_mcp_serves_status()
        .await
    {
        for mcp in mcps.iter_mut() {
            if let Some(status) = status_map.get(&mcp.name) {
                mcp.status = Some(status.clone());
            }
        }
    }
    Ok(mcps)
}

/// check the form of the MCP server config
///
/// # Arguments
/// - `name` - The name of the MCP server.
/// - `config` - The configuration of the MCP server.
///
/// # Returns
/// * `Result<(), String>` - An error message if the form is invalid, or `Ok(())` if the form is valid.
fn check_form(name: &str, config: &McpServerConfig) -> Result<(), String> {
    if name.is_empty() || config.name.is_empty() {
        return Err(t!("mcp.config.name_must_be_non_empty").to_string());
    }

    if config.protocol_type == McpProtocolType::Sse
        && config.url.clone().unwrap_or_default().is_empty()
    {
        return Err(t!("mcp.config.sse_url_must_be_non_empty").to_string());
    }

    if config.protocol_type == McpProtocolType::Stdio {
        if config.command.clone().unwrap_or_default().is_empty() {
            return Err(t!("mcp.config.stdio_command_must_be_non_empty").to_string());
        }
        if config.args.clone().unwrap_or_default().is_empty() {
            return Err(t!("mcp.config.stdio_args_must_be_non_empty").to_string());
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
#[tauri::command]
pub async fn add_mcp_server(
    main_store: State<'_, Arc<Mutex<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    name: String,
    description: String,
    config: McpServerConfig,
    disabled: bool,
) -> Result<Mcp, String> {
    check_form(&name, &config)?;

    // This variable will hold the Mcp data to be returned.
    // It's populated after the database interaction and before async operations.
    let mcp_data;
    // Scope for main_store lock: add the MCP to the database and retrieve its data.
    // User confirmed db.failed_to_lock_main_store
    {
        let mut store_guard = main_store.lock().map_err(|e| e.to_string())?;
        mcp_data = store_guard
            .add_mcp(name, description, config.clone(), disabled) // Clone config for db
            .map_err(|e| e.to_string())?;
    }

    // Register the MCP server with the function manager if it's not disabled.
    // This is an async operation, so it must happen after the main_store lock is released.
    if !mcp_data.disabled {
        chat_state
            .tool_manager
            .clone()
            .register_mcp_server(mcp_data.config.clone()) // Use the config from mcp_data
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(mcp_data)
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
    main_store: State<'_, Arc<Mutex<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    id: i64,
    name: &str,
    description: &str,
    config: McpServerConfig,
    disabled: bool,
) -> Result<Mcp, String> {
    check_form(&name, &config)?;

    // Scope for the first main_store lock: update the database.
    {
        let mut store_guard = main_store
            .lock()
            .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
        store_guard
            .update_mcp(
                id,
                name,
                description,
                config.clone(), // Clone config for the database update
                disabled,
            )
            .map_err(|e| e.to_string())?;
    } // store_guard is dropped here

    // Scope for chat_state async operations: reregister the MCP server.
    // The original `config` (McpServerConfig is Clone) and `name` (&str with sufficient lifetime)
    // are used here. `disabled` is also a direct parameter.
    {
        let fm = chat_state.tool_manager.clone();
        fm.unregister_mcp_server(name)
            .await
            .map_err(|e| e.to_string())?;
        if !disabled {
            fm.register_mcp_server(config)
                .await
                .map_err(|e| e.to_string())?;
        }
    } // fm_guard is dropped here

    // Scope for the second main_store lock: get the updated MCP data.
    {
        let store_guard = main_store
            .lock()
            .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
        store_guard
            .config
            .get_mcp_by_id(id)
            .map_err(|e| e.to_string())
    } // store_guard is dropped here
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
    main_store: State<'_, Arc<Mutex<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    id: i64,
) -> Result<(), String> {
    // Get the MCP name first to use it for unregistering after releasing the main_store lock.
    let mcp_name = {
        let store_guard = main_store
            .lock()
            .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
        let mcp = store_guard
            .config
            .get_mcp_by_id(id)
            .map_err(|e| e.to_string())?;
        mcp.name.clone()
    }; // store_guard is dropped here

    // Unregister the MCP server from function manager (async operation).
    chat_state
        .tool_manager
        .clone()
        .unregister_mcp_server(&mcp_name)
        .await
        .map_err(|e| e.to_string())?;

    // Re-acquire lock to delete from database.
    {
        let mut store_guard = main_store
            .lock()
            .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
        store_guard.delete_mcp(id).map_err(|e| e.to_string())
    } // store_guard is dropped here
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
    main_store: State<'_, Arc<Mutex<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    id: i64,
) -> Result<(), String> {
    // Get the MCP config. McpServerConfig is Clone.
    let mcp_config = {
        let store_guard = main_store
            .lock()
            .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
        let mcp = store_guard
            .config
            .get_mcp_by_id(id)
            .map_err(|e| e.to_string())?;

        if mcp.disabled == false {
            return Ok(());
        }

        mcp.config.clone()
    }; // store_guard is dropped here

    #[cfg(debug_assertions)]
    {
        log::debug!("enabling mcp server, mcp_config: {:?}", mcp_config);
    }

    // Start the MCP server using the function manager (async operation).
    chat_state
        .tool_manager
        .clone()
        .start_mcp_server(mcp_config)
        .await
        .map_err(|e| e.to_string())?;

    #[cfg(debug_assertions)]
    {
        log::debug!("mcp server started");
    }

    // Re-acquire lock to update the database after server start up.
    let mut store_guard = main_store
        .lock()
        .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
    store_guard
        .change_mcp_status(id, false)
        .map_err(|e| e.to_string())?;

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
    main_store: State<'_, Arc<Mutex<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    id: i64,
) -> Result<(), String> {
    // Get the MCP name.
    let mcp_name = {
        let store_guard = main_store
            .lock()
            .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
        let mcp = store_guard
            .config
            .get_mcp_by_id(id)
            .map_err(|e| e.to_string())?;
        if mcp.disabled == true {
            return Ok(());
        }

        mcp.name.clone()
    }; // store_guard is dropped here

    // Stop the MCP server using the function manager (async operation).
    chat_state
        .tool_manager
        .clone()
        .stop_mcp_server(&mcp_name)
        .await
        .map_err(|e| e.to_string())?;

    // Re-acquire lock to update the database after server stop.
    let mut store_guard = main_store
        .lock()
        .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
    store_guard
        .change_mcp_status(id, true)
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// Restart an MCP server
///
/// Restarts the specified MCP server.
///
/// # Arguments
/// - `main_store` - The state of the main application store.
/// - `chat_state` - The state of the chat system.
/// - `id` - The ID of the MCP server to restart.
///
/// # Returns
/// * `Result<(), String>` - Ok if successful, or an error message.
#[tauri::command]
pub async fn restart_mcp_server(
    main_store: State<'_, Arc<Mutex<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    id: i64,
) -> Result<(), String> {
    let mcp_config = {
        let store_guard = main_store
            .lock()
            .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
        let mcp = store_guard
            .config
            .get_mcp_by_id(id)
            .map_err(|e| e.to_string())?;

        if mcp.disabled == false {
            return Ok(());
        }

        mcp.config.clone()
    };

    let fm = chat_state.tool_manager.clone();
    fm.stop_mcp_server(mcp_config.name.as_str())
        .await
        .map_err(|e| e.to_string())?;

    fm.start_mcp_server(mcp_config)
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
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
    main_store: State<'_, Arc<Mutex<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    id: i64,
) -> Result<Vec<MCPToolDeclaration>, String> {
    // Get the MCP name.
    let mcp_name = {
        let store_guard = main_store
            .lock()
            .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
        let mcp = store_guard
            .config
            .get_mcp_by_id(id)
            .map_err(|e| e.to_string())?;
        mcp.name.clone()
    }; // store_guard is dropped here

    // Get tools from the MCP server using the function manager (async operation).
    chat_state
        .tool_manager
        .clone()
        .get_mcp_server_tools(&mcp_name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_mcp_tool_status(
    main_store: State<'_, Arc<Mutex<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    id: i64,
    tool_name: &str,
    disabled: bool,
) -> Result<Mcp, String> {
    let mcp = {
        let mut store_guard = main_store
            .lock()
            .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
        let mut mcp = store_guard
            .config
            .get_mcp_by_id(id)
            .map_err(|e| e.to_string())?;

        let disabled_tools = mcp.config.disabled_tools.get_or_insert_with(HashSet::new);
        if disabled {
            disabled_tools.insert(tool_name.to_string());
        } else {
            disabled_tools.remove(tool_name);
        }

        store_guard
            .update_mcp(
                id,
                &mcp.name,
                &mcp.description,
                mcp.config.clone(),
                mcp.disabled,
            )
            .map_err(|e| e.to_string())?
    };

    chat_state
        .tool_manager
        .disable_mcp_tool(mcp.name.as_str(), tool_name, disabled)
        .await
        .map_err(|e| e.to_string())?;

    Ok(mcp)
}
