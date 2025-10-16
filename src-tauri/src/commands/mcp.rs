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
    mcp::client::{McpProtocolType, McpServerConfig},
    mcp::McpError,
};
use rust_i18n::t;
use std::{
    collections::HashSet,
    sync::{Arc, RwLock},
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
    main_store: State<'_, Arc<RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
) -> Result<Vec<Mcp>> {
    // First, get the MCPs from the main_store.
    let mut mcps = {
        let store_guard = main_store.read()?;
        store_guard.config.get_mcps()
    };

    // Get the status of each MCP server and update the status field
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
fn check_form(name: &str, config: &McpServerConfig) -> Result<()> {
    if name.is_empty() || config.name.is_empty() {
        return Err(AppError::Mcp(McpError::ClientConfigError(
            t!("mcp.config.name_must_be_non_empty").to_string(),
        )));
    }

    if config.protocol_type == McpProtocolType::Sse
        && config.url.clone().unwrap_or_default().is_empty()
    {
        return Err(AppError::Mcp(McpError::ClientConfigError(
            t!("mcp.config.sse_url_must_be_non_empty").to_string(),
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
#[tauri::command]
pub async fn add_mcp_server(
    main_store: State<'_, Arc<RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    name: String,
    description: String,
    config: McpServerConfig,
    disabled: bool,
) -> Result<Mcp> {
    check_form(&name, &config)?;

    let mcp_data = {
        let mut store_guard = main_store.write()?;
        store_guard.add_mcp(name, description, config.clone(), disabled)?
    };

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
    main_store: State<'_, Arc<RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    id: i64,
    name: &str,
    description: &str,
    config: McpServerConfig,
    disabled: bool,
) -> Result<Mcp> {
    check_form(&name, &config)?;

    {
        let mut store_guard = main_store.write()?;
        store_guard.update_mcp(id, name, description, config.clone(), disabled)?;
    }

    let updated_mcp = {
        let store_guard = main_store.read()?;
        store_guard.config.get_mcp_by_id(id)?
    };

    {
        let fm = chat_state.tool_manager.clone();
        let server_name = name.to_string();
        let config_clone = config.clone();

        tokio::spawn(async move {
            if let Err(e) = fm.unregister_mcp_server(&server_name).await {
                log::warn!(
                    "Failed to unregister MCP server '{}' during update: {}",
                    server_name,
                    e
                );
            }

            if !disabled {
                if let Err(e) = fm.register_mcp_server(config_clone).await {
                    log::error!(
                        "Failed to restart MCP server '{}' after update: {}",
                        server_name,
                        e
                    );
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
    main_store: State<'_, Arc<RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    id: i64,
) -> Result<()> {
    let mcp_name = {
        let store_guard = main_store.read()?;
        let mcp = store_guard.config.get_mcp_by_id(id)?;
        mcp.name.clone()
    };

    {
        let mut store_guard = main_store.write()?;
        store_guard.delete_mcp(id)?;
    }

    let tool_manager = chat_state.tool_manager.clone();
    let server_name = mcp_name.clone();

    tokio::spawn(async move {
        if let Err(e) = tool_manager.unregister_mcp_server(&server_name).await {
            log::error!(
                "Failed to unregister MCP server '{}' after deleting from database: {}",
                server_name,
                e
            );
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
    main_store: State<'_, Arc<RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    id: i64,
) -> Result<()> {
    {
        let mut ops = chat_state.tool_manager.ops_in_progress.lock().await;
        if !ops.insert(id) {
            return Err(AppError::Mcp(McpError::StateChangeFailed(
                t!("mcp.op_in_progress_error").to_string(),
            )));
        }
    }

    let server_info = {
        let store_guard = main_store.read()?;
        store_guard
            .config
            .get_mcp_by_id(id)
            .map(|mcp| (mcp.config.clone(), !mcp.disabled))
            .map_err(|e| AppError::Db(e))
    };

    let mcp_config = match server_info {
        Ok((config, already_enabled)) => {
            if already_enabled {
                chat_state
                    .tool_manager
                    .ops_in_progress
                    .lock()
                    .await
                    .remove(&id);
                return Ok(());
            }
            config
        }
        Err(e) => {
            chat_state
                .tool_manager
                .ops_in_progress
                .lock()
                .await
                .remove(&id);
            return Err(e);
        }
    };

    #[cfg(debug_assertions)]
    {
        log::debug!("enabling mcp server, mcp_config: {:?}", mcp_config);
    }

    {
        let mut store_guard = main_store.write()?;
        store_guard.change_mcp_status(id, false)?;
    }

    let tool_manager = chat_state.tool_manager.clone();

    tokio::spawn(async move {
        #[cfg(debug_assertions)]
        {
            log::debug!("enabling mcp server, mcp_config: {:?}", mcp_config);
        }

        if let Err(e) = tool_manager
            .clone()
            .start_mcp_server(mcp_config.clone())
            .await
        {
            log::error!(
                "Failed to start MCP server '{}' after enabling: {}",
                mcp_config.name,
                e
            );
        } else {
            log::info!(
                "MCP server '{}' started successfully after being enabled",
                mcp_config.name
            );
            #[cfg(debug_assertions)]
            {
                log::debug!("mcp server started");
            }
        }
        tool_manager.ops_in_progress.lock().await.remove(&id);
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
    main_store: State<'_, Arc<RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    id: i64,
) -> Result<()> {
    {
        let mut ops = chat_state.tool_manager.ops_in_progress.lock().await;
        if !ops.insert(id) {
            return Err(AppError::Mcp(McpError::StateChangeFailed(
                t!("mcp.op_in_progress_error").to_string(),
            )));
        }
    }

    let server_info = {
        let store_guard = main_store.read()?;
        store_guard
            .config
            .get_mcp_by_id(id)
            .map(|mcp| (mcp.name.clone(), mcp.disabled))
            .map_err(AppError::Db)
    };

    let mcp_name = match server_info {
        Ok((name, disabled)) => {
            if disabled {
                chat_state
                    .tool_manager
                    .ops_in_progress
                    .lock()
                    .await
                    .remove(&id);
                return Ok(());
            }
            name
        }
        Err(e) => {
            chat_state
                .tool_manager
                .ops_in_progress
                .lock()
                .await
                .remove(&id);
            return Err(e);
        }
    };

    {
        let mut store_guard = main_store.write()?;
        store_guard.change_mcp_status(id, true)?;
    }

    let tool_manager = chat_state.tool_manager.clone();

    tokio::spawn(async move {
        log::info!("Starting to stop MCP server '{}'", mcp_name);

        match tool_manager.stop_mcp_server(&mcp_name).await {
            Ok(_) => {
                log::info!(
                    "MCP server '{}' stopped successfully after being disabled",
                    mcp_name
                );
            }
            Err(e) => {
                log::error!(
                    "Failed to stop MCP server '{}' after disabling: {}",
                    mcp_name,
                    e
                );
            }
        }

        log::info!("Finished stopping MCP server '{}'", mcp_name);
        tool_manager.ops_in_progress.lock().await.remove(&id);
    });

    Ok(())
}

#[tauri::command]
pub async fn restart_mcp_server(
    main_store: State<'_, Arc<RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    id: i64,
) -> Result<()> {
    {
        let mut ops = chat_state.tool_manager.ops_in_progress.lock().await;
        if !ops.insert(id) {
            return Err(AppError::Mcp(McpError::StateChangeFailed(
                t!("mcp.op_in_progress_error").to_string(),
            )));
        }
    }

    let server_info = {
        let store_guard = main_store.read()?;
        store_guard
            .config
            .get_mcp_by_id(id)
            .map(|mcp| (mcp.name.clone(), mcp.disabled))
            .map_err(AppError::Db)
    };

    let server_name = match server_info {
        Ok((name, disabled)) => {
            if disabled {
                chat_state
                    .tool_manager
                    .ops_in_progress
                    .lock()
                    .await
                    .remove(&id);
                return Err(AppError::Mcp(McpError::StateChangeFailed(
                    t!("mcp.error.cannot_restart_disabled_server").to_string(),
                )));
            }
            name
        }
        Err(e) => {
            chat_state
                .tool_manager
                .ops_in_progress
                .lock()
                .await
                .remove(&id);
            return Err(e);
        }
    };

    let tool_manager = chat_state.tool_manager.clone();
    let main_store_clone = main_store.inner().clone();

    tokio::spawn(async move {
        if let Err(e) = tool_manager.stop_mcp_server(&server_name).await {
            log::warn!(
                "Failed to stop MCP server '{}' during restart: {}",
                server_name,
                e
            );
        }

        let config_to_start: Option<McpServerConfig> = {
            let store_guard = main_store_clone.read().map_err(|e| {
                AppError::Db(crate::db::StoreError::LockError(
                    t!("db.failed_to_lock_main_store", error = e.to_string()).to_string(),
                ))
            });
            match store_guard {
                Ok(guard) => match guard.config.get_mcp_by_id(id) {
                    Ok(mcp) => {
                        if mcp.disabled {
                            log::info!("MCP server '{}' was disabled before it could be restarted. Aborting restart.", server_name);
                            None
                        } else {
                            Some(mcp.config)
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to get MCP config for id {} during restart: {}. Aborting restart.", id, e);
                        None
                    }
                },
                Err(e) => {
                    log::error!("Failed to lock main_store during restart: {}", e);
                    None
                }
            }
        };

        if let Some(mcp_config) = config_to_start {
            if let Err(e) = tool_manager.clone().start_mcp_server(mcp_config).await {
                log::error!(
                    "Failed to start MCP server '{}' during restart: {}",
                    server_name,
                    e
                );
            } else {
                log::info!("MCP server '{}' restarted successfully", server_name);
            }
        }

        tool_manager.ops_in_progress.lock().await.remove(&id);
    });

    Ok(())
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
    chat_state: State<'_, Arc<ChatState>>,
    main_store: State<'_, Arc<RwLock<MainStore>>>,
    id: i64,
) -> Result<()> {
    {
        let mut ops = chat_state.tool_manager.ops_in_progress.lock().await;
        if !ops.insert(id) {
            return Err(AppError::Mcp(McpError::StateChangeFailed(
                t!("mcp.op_in_progress_error").to_string(),
            )));
        }
    }

    let server_info = {
        let store_guard = main_store.read()?;
        store_guard
            .config
            .get_mcp_by_id(id)
            .map(|mcp| (mcp.name.clone(), mcp.disabled))
            .map_err(AppError::Db)
    };

    let server_name = match server_info {
        Ok((name, disabled)) => {
            if disabled {
                chat_state
                    .tool_manager
                    .ops_in_progress
                    .lock()
                    .await
                    .remove(&id);
                return Err(AppError::Mcp(McpError::StateChangeFailed(
                    t!("mcp.error.cannot_refresh_disabled_server").to_string(),
                )));
            }
            name
        }
        Err(e) => {
            chat_state
                .tool_manager
                .ops_in_progress
                .lock()
                .await
                .remove(&id);
            return Err(e);
        }
    };

    let tool_manager = chat_state.tool_manager.clone();
    log::info!("Triggered tool refresh for MCP server '{}'", server_name);

    tokio::spawn(async move {
        if let Err(e) = tool_manager.refresh_mcp_server_tools(&server_name).await {
            log::error!(
                "Error refreshing MCP server '{}' in background: {}",
                server_name,
                e
            );
        }
        tool_manager.ops_in_progress.lock().await.remove(&id);
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
    main_store: State<'_, Arc<RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    id: i64,
) -> Result<Vec<MCPToolDeclaration>> {
    let mcp_name = {
        let store_guard = main_store.read()?;
        let mcp = store_guard.config.get_mcp_by_id(id)?;
        mcp.name.clone()
    };

    let tools_result = chat_state
        .tool_manager
        .clone()
        .get_mcp_server_tools(&mcp_name)
        .await;
    tools_result.map_err(|e| AppError::Mcp(McpError::NotFound(e.to_string())))
}

#[tauri::command]
pub async fn update_mcp_tool_status(
    main_store: State<'_, Arc<RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    id: i64,
    tool_name: &str,
    disabled: bool,
) -> Result<Mcp> {
    let mcp = {
        let mut store_guard = main_store.write()?;
        let mut mcp = store_guard.config.get_mcp_by_id(id)?;

        let disabled_tools = mcp.config.disabled_tools.get_or_insert_with(HashSet::new);
        if disabled {
            disabled_tools.insert(tool_name.to_string());
        } else {
            disabled_tools.remove(tool_name);
        }

        store_guard.update_mcp(
            id,
            &mcp.name,
            &mcp.description,
            mcp.config.clone(),
            mcp.disabled,
        )?
    };

    let disable_result = chat_state
        .tool_manager
        .disable_mcp_tool(mcp.name.as_str(), tool_name, disabled)
        .await;
    disable_result.map_err(|e| AppError::Mcp(McpError::General(e.to_string())))?;

    Ok(mcp)
}
