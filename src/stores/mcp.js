import { defineStore } from 'pinia';
import { ref, reactive } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'

import { sendSyncState } from '@/libs/sync.js'

/**
 * @typedef {Object} McpServerConfigEnv
 * @property {string} 0 - Environment variable name
 * @property {string} 1 - Environment variable value
 */

/**
 * @typedef {Object} McpServerConfig
 * @property {string} name - Name of the MCP server (must match the MCP's own name for registration)
 * @property {'sse' | 'stdio'} type - Protocol type
 * @property {string | null} [url] - URL for SSE protocol
 * @property {string | null} [bearer_token] - Bearer token for SSE
 * @property {string | null} [proxy] - Proxy URL
 * @property {string | null} [command] - Command for Stdio protocol
 * @property {string[] | null} [args] - Arguments for Stdio protocol
 * @property {McpServerConfigEnv[] | null} [env] - Environment variables for Stdio protocol
 * @property {string[] | null} [disabled_tools] - List of initially disabled tools within this server config
 */

/**
 * @typedef {Object} McpServer
 * @property {number} id - Unique identifier for the MCP record
 * @property {string} name - Human-readable name of the MCP configuration
 * @property {string} description - Detailed description of the MCP configuration
 * @property {McpServerConfig} config - Server configuration
 * @property {boolean} disabled - Whether this MCP configuration is disabled
 * @property {'running' | 'stopped' | 'error' | string | null} [status] - Current status of the MCP server
 */

/**
 * @typedef {Object} MCPToolDeclaration
 * @property {string} name - Name of the tool
 * @property {string} description - Description of the tool
 * @property {Object} input_schema - JSON schema for the tool's input
 */

const label = getCurrentWebviewWindow().label

export const useMcpStore = defineStore('mcp', () => {
  /** @type {import('vue').Ref<McpServer[]>} */
  const servers = ref([]);
  /** @type {import('vue').Ref<Record<number, MCPToolDeclaration[]>>} */
  const serverTools = ref({});
  /**
   * ui status for mcp manager page
   * @type {import('vue').Ref<Record<number, { expanded: boolean, loading: boolean }>>}
  */
  const serverUiStates = ref({});

  const loading = ref(false);
  const error = ref(null);

  const _handleError = async (err) => {
    error.value = err.message || String(err);
    loading.value = false;
    console.error('MCP Store Error:', error.value);
    throw err; // Re-throw for component-level handling if needed
  };

  /**
   * Fetches all MCP servers from the backend and updates the local state.
   */
  const fetchMcpServers = async () => {
    loading.value = true;
    error.value = null;
    try {
      const fetchedServers = await invoke('list_mcp_servers');
      servers.value = fetchedServers.map(server => {
        // Ensure server.config exists and disabled_tools is an array
        const config = server.config || {}; // Defensive, though McpServer type implies config exists
        const disabled_tools = Array.isArray(config.disabled_tools)
          ? config.disabled_tools
          : []; // Default to empty array if null or not an array
        return {
          ...server,
          config: {
            ...config, // Spread original config properties
            disabled_tools: disabled_tools,
          },
        };
      });
      console.debug(servers.value)
    } catch (err) {
      await _handleError(err);
    } finally {
      loading.value = false;
    }
  };

  /**
   * Adds a new MCP server.
   * @param {Object} payload
   * @param {string} payload.name
   * @param {string} payload.description
   * @param {McpServerConfig} payload.config
   * @param {boolean} payload.disabled
   * @returns {Promise<McpServer | undefined>} The added server data.
   */
  const addMcpServer = async (payload) => {
    loading.value = true;
    error.value = null;
    try {
      const newServer = await invoke('add_mcp_server', payload);
      // Local state update handled by handleSyncStateUpdate or by receiving its own sync event
      // For direct local update: handleSyncStateUpdate({ event: 'add', data: newServer });
      sendSyncState('mcp', label, { event: 'add', data: newServer });
      return newServer;
    } catch (err) {
      await _handleError(err);
    } finally {
      loading.value = false;
    }
  };

  /**
   * Updates an existing MCP server.
   * @param {Object} payload
   * @param {number} payload.id
   * @param {string} payload.name
   * @param {string} payload.description
   * @param {McpServerConfig} payload.config
   * @param {boolean} payload.disabled
   * @returns {Promise<McpServer | undefined>} The updated server data.
   */
  const updateMcpServer = async (payload) => {
    loading.value = true;
    error.value = null;
    try {
      const updatedServer = await invoke('update_mcp_server', payload);
      // Local state update handled by handleSyncStateUpdate or by receiving its own sync event
      // For direct local update: handleSyncStateUpdate({ event: 'update', data: updatedServer });

      sendSyncState('mcp', label, { event: 'update', data: updatedServer });

      // remove tools from serverTools when tool is disabled
      if (payload.disabled) {
        delete serverTools.value[payload.id];
      }
      return updatedServer;
    } catch (err) {
      await _handleError(err);
    } finally {
      loading.value = false;
    }
  };

  /**
 * Updates an existing MCP server.
 * @param {Object} payload
 * @param {number} payload.id [optional]
 * @param {string} payload.name
 * @param {string} payload.description
 * @param {McpServerConfig} payload.config
 * @param {boolean} payload.disabled
 * @returns {Promise<McpServer | undefined>} The updated server data.
 */
  const saveMcpServer = async (payload) => {
    if (payload.id) {
      return updateMcpServer(payload);
    } else {
      return addMcpServer(payload);
    }
  }

  /**
   * Deletes an MCP server.
   * @param {number} id - The ID of the MCP server to delete.
   */
  const deleteMcpServer = async (id) => {
    loading.value = true;
    error.value = null;
    try {
      await invoke('delete_mcp_server', { id });
      // Local state update handled by handleSyncStateUpdate or by receiving its own sync event
      // For direct local update: handleSyncStateUpdate({ event: 'delete', data: { id } });
      sendSyncState('mcp', label, { event: 'delete', data: { id } });
    } catch (err) {
      await _handleError(err);
    } finally {
      loading.value = false;
    }
  };

  /**
   * Starts (connects to) an MCP server.
   * @param {number} id - The ID of the MCP server to start.
   */
  const enableMcpServer = async (id) => {
    loading.value = true;
    error.value = null;
    try {
      await invoke('enable_mcp_server', { id });
      // Local state update handled by handleSyncStateUpdate or by receiving its own sync event
      // For direct local update: handleSyncStateUpdate({ event: 'update', data: { id, disabled: false } });
      sendSyncState('mcp', label, { event: 'update', data: { id, disabled: false } });
    } catch (err) {
      await _handleError(err);
    } finally {
      loading.value = false;
    }
  };

  /**
   * Stops (disconnects from) an MCP server.
   * @param {number} id - The ID of the MCP server to stop.
   */
  const disableMcpServer = async (id) => {
    loading.value = true;
    error.value = null;
    try {
      await invoke('disable_mcp_server', { id });
      // Local state update handled by handleSyncStateUpdate or by receiving its own sync event
      // For direct local update: handleSyncStateUpdate({ event: 'update', data: { id, disabled: true } });
      sendSyncState('mcp', label, { event: 'update', data: { id, disabled: true } });
    } catch (err) {
      await _handleError(err);
    } finally {
      loading.value = false;
    }
  };

  const restartMcpServer = async (id) => {
    loading.value = true;
    error.value = null;
    try {
      await invoke('restart_mcp_server', { id });

      // Restart might change status, but we don't get the new status back synchronously here.
      // Rely on status updates pushed from backend or a periodic refresh if needed.
    } catch (err) {
      await _handleError(err);
    } finally {
      loading.value = false;
    }
  };

  /**
   * Fetches the tools provided by a specific MCP server.
   * @param {number} serverId - The ID of the MCP server.
   */
  const fetchMcpServerTools = async (serverId) => {
    loading.value = true;
    error.value = null;
    try {
      const tools = await invoke('get_mcp_server_tools', { id: serverId });
      serverTools.value = {
        ...serverTools.value,
        [serverId]: tools,
      };
      console.log(serverTools.value)
    } catch (err) {
      await _handleError(err);
    } finally {
      loading.value = false;
    }
  };

  /**
   * Toggles the disabled status of a specific tool for a server by updating the server's configuration.
   * This function modifies the server's `config.disabled_tools` array and calls the backend
   * to persist the change. It does NOT directly modify the `serverTools` cache, which
   * is assumed to store the full list of declared tools.
   * @param {number} serverId - The ID of the MCP server.
   * @param {MCPToolDeclaration} tool - The tool object whose disabled status is being toggled.
   * @returns {Promise<void>} A promise that resolves when the server configuration is updated.
   */
  const toggleDisableTool = async (serverId, tool) => {
    // Find the server in the servers list
    const serverIndex = servers.value.findIndex(s => s.id === serverId);
    if (serverIndex === -1) {
      console.error(`Server with ID ${serverId} not found.`);
      throw new Error(`Server with ID ${serverId} not found.`);
    }

    // serverToUpdate is a shallow copy. serverToUpdate.config refers to the same object in the store.
    // With the change in fetchMcpServers, serverToUpdate.config.disabled_tools is guaranteed to be an array.

    try {
      const currentServerInStore = servers.value[serverIndex];
      if (!currentServerInStore) {
        throw new Error(`Server with ID ${serverId} disappeared unexpectedly.`);
      }

      const toolName = tool.name;
      // Determine the new desired state for the tool
      const currentDisabledTools = currentServerInStore.config?.disabled_tools || [];
      const isCurrentlyDisabled = currentDisabledTools.includes(toolName);
      const newDisabledStateForTool = !isCurrentlyDisabled; // This is what we tell the backend

      // Call the backend to update the server configuration
      // We expect this call to primarily affect the tool's status within the server's config.
      const backendUpdateResult = await invoke('update_mcp_tool_status', { id: serverId, toolName: toolName, disabled: newDisabledStateForTool });

      if (backendUpdateResult) {
        // Preserve the existing server state (like 'status') and merge the backend update.
        // backendUpdateResult might only contain partial data (e.g., just the updated config).
        const newConfig = {
          ...(currentServerInStore.config || {}), // Start with current config
          ...(backendUpdateResult.config || {}),   // Overlay with response's config fields
        };

        // Ensure disabled_tools is correctly sourced from backendUpdateResult or inferred.
        // The most reliable is if backendUpdateResult.config.disabled_tools is the new list.
        const backendConfig = backendUpdateResult.config || {};
        const backendDisabledTools = Array.isArray(backendConfig.disabled_tools)
          ? backendConfig.disabled_tools
          // Fallback to current server's disabled_tools if not in backend response,
          // or an empty array if that's also missing.
          : (currentServerInStore.config?.disabled_tools || []);
        newConfig.disabled_tools = backendDisabledTools;
        // Local state update handled by handleSyncStateUpdate or by receiving its own sync event
        // For direct local update: handleSyncStateUpdate({ event: 'toggleToolStatus', data: { id: serverId, disabled_tools: newConfig.disabled_tools } });
        // Send sync state only if the backend update was processed and local state updated
        sendSyncState('mcp', label, { event: 'toggleToolStatus', data: { id: serverId, disabled_tools: newConfig.disabled_tools } });
      }

      return servers.value[serverIndex]; // Return the updated server state from the store
    } catch (err) {
      console.error(`Failed to toggle tool "${toolName}" state for server ID ${serverId} via backend:`, err); // Use toolName for consistency
      throw err; // Re-throw the error
    }
  }

  /**
   * Updates the status of a specific MCP server.
   * This is typically called by the sync state listener for 'mcp_status_changed' events.
   * @param {string} serverName - The name of the server whose status is to be updated.
   * @param {string | { error: string }} status - The new status of the server.
   */
  const updateServerStatus = (serverName, status) => {
    const index = servers.value.findIndex(s => s.name === serverName);
    if (index !== -1) {
      // Create a new object to ensure reactivity update is picked up by Vue
      servers.value.splice(index, 1, { ...servers.value[index], status: status });
      console.debug(`MCP Store: Updated status for server "${serverName}" to`, status);
    }
  };


  fetchMcpServers();

  // Helper to get or initialize UI state for a server, meant for internal store use or direct component use
  const getOrInitServerUiState = (serverId) => {
    if (!serverUiStates.value[serverId]) {
      serverUiStates.value[serverId] = reactive({ expanded: false, loading: false });
    }
    return serverUiStates.value[serverId];
  };

  /**
   * Handles synchronization state updates received from other windows.
   * Performs targeted updates on the local state based on the event type.
   * This method is intended to be called by the sync state listener in App.vue.
   * @param {Object} metadata - The metadata payload from sendSyncState.
   * @param {'add' | 'update' | 'delete' | 'toggleToolStatus'} metadata.event - The type of update event.
   * @param {any} metadata.data - The data associated with the event (e.g., server object, ID, disabled_tools array).
   */
  const handleSyncStateUpdate = (metadata) => {
    if (!metadata || !metadata.event) {
      console.warn('Received invalid sync state metadata:', metadata);
      return;
    }

    const { event, data } = metadata;

    switch (event) {
      case 'add': {
        // Add new server if it doesn't exist
        if (data && data.id && !servers.value.some(s => s.id === data.id)) {
          // Ensure config and disabled_tools are correctly formatted before adding
          const newServerData = {
            ...data,
            config: {
              ...(data.config || {}),
              disabled_tools: Array.isArray(data.config?.disabled_tools) ? data.config.disabled_tools : [],
            },
          };
          servers.value.push(newServerData);
          console.debug('MCP Store: Added server via sync', data.id);
        }
        break;
      }
      case 'update': {
        // Update existing server by merging data
        if (data && data.id) {
          const index = servers.value.findIndex(s => s.id === data.id);
          if (index !== -1) {
            const currentServer = servers.value[index];
            // Destructure data to separate status. Status is managed by updateServerStatus
            // and should not be overwritten by general update events unless specifically intended.
            const { status: incomingStatus, ...otherDataFromEvent } = data;

            const updatedServer = {
              ...currentServer,        // Preserve currentServer properties, including its up-to-date status
              ...otherDataFromEvent,   // Apply other updates from the event data (excluding status from data)
              config: {         // Carefully merge config
                ...(currentServer.config || {}), // Start with current config
                ...(data.config || {}),          // Overlay with incoming config fields
              },
            };
            // Ensure disabled_tools in the merged config is correctly an array
            updatedServer.config.disabled_tools = Array.isArray(data.config?.disabled_tools)
              ? data.config.disabled_tools
              : (Array.isArray(currentServer.config?.disabled_tools) ? currentServer.config.disabled_tools : []);

            servers.value.splice(index, 1, updatedServer); // Replace item to trigger reactivity
            console.debug('MCP Store: Updated server via sync', data.id);
          }
        }
        break;
      }
      case 'delete': {
        // Remove server by ID
        if (data && data.id) {
          servers.value = servers.value.filter(s => s.id !== data.id);
          // Clean up associated states
          delete serverTools.value[data.id];
          delete serverUiStates.value[data.id];
          console.debug('MCP Store: Deleted server via sync', data.id);
        }
        break;
      }
      case 'toggleToolStatus': {
        // Update only disabled_tools for a server
        if (data && data.id && Array.isArray(data.disabled_tools)) {
          const index = servers.value.findIndex(s => s.id === data.id);
          if (index !== -1) {
            const currentServer = servers.value[index];
            // Create a new object for the server with updated config to ensure reactivity
            const updatedServer = {
              ...currentServer,
              config: {
                ...(currentServer.config || {}), // Preserve other config fields
                disabled_tools: data.disabled_tools,
              },
            };
            servers.value.splice(index, 1, updatedServer); // Replace item
            // Vue 3 reactivity should handle this direct modification.
            console.debug('MCP Store: Toggled tool status via sync for server', data.id);
          }
        }
        break;
      }
      default:
        console.warn('MCP Store: Received unknown sync state event type:', event);
        break;
    }
  };

  return {
    servers,
    serverTools,
    loading,
    error,
    fetchMcpServers,
    addMcpServer,
    updateMcpServer,
    saveMcpServer,
    deleteMcpServer,
    enableMcpServer,
    disableMcpServer,
    restartMcpServer,
    fetchMcpServerTools,
    toggleDisableTool,
    handleSyncStateUpdate,
    serverUiStates,
    getOrInitServerUiState,
    updateServerStatus
  };
});