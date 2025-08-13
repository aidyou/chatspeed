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
    // We spawn this as a background task to avoid blocking the add operation.
    // If MCP server startup fails, the data is still saved in the database.
    if !mcp_data.disabled {
        let tool_manager = chat_state.tool_manager.clone();
        let config = mcp_data.config.clone();
        let server_name = config.name.clone();

        tokio::spawn(async move {
            if let Err(e) = tool_manager.register_mcp_server(config).await {
                log::error!(
                    "Failed to start MCP server '{}' after adding to database: {}",
                    server_name,
                    e
                );
                // Note: The server data is still in the database and can be manually started later
            } else {
                log::info!(
                    "MCP server '{}' started successfully after being added to database",
                    server_name
                );
            }
        });
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

    // Get the updated MCP data first before async operations
    let updated_mcp = {
        let store_guard = main_store
            .lock()
            .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
        store_guard
            .config
            .get_mcp_by_id(id)
            .map_err(|e| e.to_string())?
    }; // store_guard is dropped here

    // Scope for chat_state async operations: reregister the MCP server.
    // We spawn this as a background task to avoid blocking the update operation.
    // If MCP server restart fails, the data is still updated in the database.
    {
        let fm = chat_state.tool_manager.clone();
        let server_name = name.to_string();
        let config_clone = config.clone();

        tokio::spawn(async move {
            // First unregister the old server
            if let Err(e) = fm.unregister_mcp_server(&server_name).await {
                log::warn!(
                    "Failed to unregister MCP server '{}' during update: {}",
                    server_name,
                    e
                );
            }

            // Then register the new configuration if not disabled
            if !disabled {
                if let Err(e) = fm.register_mcp_server(config_clone).await {
                    log::error!(
                        "Failed to restart MCP server '{}' after update: {}",
                        server_name,
                        e
                    );
                    // Note: The server data is still updated in the database and can be manually started later
                } else {
                    log::info!(
                        "MCP server '{}' restarted successfully after update",
                        server_name
                    );
                }
            }
        });
    }

    Ok(updated_mcp)
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

    // Delete from database first
    {
        let mut store_guard = main_store
            .lock()
            .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
        store_guard.delete_mcp(id).map_err(|e| e.to_string())?;
    } // store_guard is dropped here

    // Unregister the MCP server from function manager (async operation).
    // We spawn this as a background task to avoid blocking the delete operation.
    // The server data is already deleted from the database.
    let tool_manager = chat_state.tool_manager.clone();
    let server_name = mcp_name.clone();

    tokio::spawn(async move {
        if let Err(e) = tool_manager.unregister_mcp_server(&server_name).await {
            log::error!(
                "Failed to unregister MCP server '{}' after deleting from database: {}",
                server_name,
                e
            );
            // Note: The server data is deleted from the database but may still be running
        } else {
            log::info!(
                "MCP server '{}' unregistered successfully after being deleted",
                server_name
            );
        }
    });

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

    // Update the database status first to mark it as enabled
    {
        let mut store_guard = main_store
            .lock()
            .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
        store_guard
            .change_mcp_status(id, false)
            .map_err(|e| e.to_string())?;
    } // store_guard is dropped here

    // Start the MCP server using the function manager (async operation).
    // We spawn this as a background task to avoid blocking the enable operation.
    // The server is already marked as enabled in the database.
    let tool_manager = chat_state.tool_manager.clone();
    let server_name = mcp_config.name.clone();

    tokio::spawn(async move {
        #[cfg(debug_assertions)]
        {
            log::debug!("enabling mcp server, mcp_config: {:?}", mcp_config);
        }

        if let Err(e) = tool_manager.start_mcp_server(mcp_config).await {
            log::error!(
                "Failed to start MCP server '{}' after enabling: {}",
                server_name,
                e
            );
            // Note: The server is marked as enabled in the database but failed to start
            // Users can try to restart it manually or check the configuration
        } else {
            log::info!(
                "MCP server '{}' started successfully after being enabled",
                server_name
            );
            #[cfg(debug_assertions)]
            {
                log::debug!("mcp server started");
            }
        }
    });

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

    // Update the database status first to mark it as disabled
    {
        let mut store_guard = main_store
            .lock()
            .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
        store_guard
            .change_mcp_status(id, true)
            .map_err(|e| e.to_string())?;
    } // store_guard is dropped here

    // Stop the MCP server using the function manager (async operation).
    // We spawn this as a background task to avoid blocking the disable operation.
    // The server is already marked as disabled in the database.
    let tool_manager = chat_state.tool_manager.clone();
    let server_name = mcp_name.clone();

    tokio::spawn(async move {
        log::info!("Starting to stop MCP server '{}'", server_name);

        match tool_manager.stop_mcp_server(&server_name).await {
            Ok(_) => {
                log::info!(
                    "MCP server '{}' stopped successfully after being disabled",
                    server_name
                );
            }
            Err(e) => {
                log::error!(
                    "Failed to stop MCP server '{}' after disabling: {}",
                    server_name,
                    e
                );
                // Note: The server is marked as disabled in the database but may still be running
                // Users can try to restart it manually or check the configuration
            }
        }

        log::info!("Finished stopping MCP server '{}'", server_name);
    });

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

        // Only restart if the server is enabled (not disabled)
        if mcp.disabled {
            return Err(
                "Cannot restart a disabled MCP server. Please enable it first.".to_string(),
            );
        }

        mcp.config.clone()
    };

    // Restart the MCP server in a background task to avoid blocking
    let fm = chat_state.tool_manager.clone();
    let server_name = mcp_config.name.clone();

    tokio::spawn(async move {
        // First stop the server
        if let Err(e) = fm.stop_mcp_server(&server_name).await {
            log::warn!(
                "Failed to stop MCP server '{}' during restart: {}",
                server_name,
                e
            );
        }

        // Then start it again (this will automatically send "Starting" status)
        if let Err(e) = fm.start_mcp_server(mcp_config).await {
            log::error!(
                "Failed to start MCP server '{}' during restart: {}",
                server_name,
                e
            );
        } else {
            log::info!("MCP server '{}' restarted successfully", server_name);
        }
    });

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
