import { defineStore } from 'pinia';
import { ref } from 'vue';
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
      servers.value.push(newServer);

      sendSyncState('mcp', label)
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
      const index = servers.value.findIndex(s => s.id === payload.id);
      if (index !== -1) {
        servers.value[index] = updatedServer;
      } else {
        await fetchMcpServers();
      }

      sendSyncState('mcp', label)

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
      // remove tools from serverTools
      delete serverTools.value[id];

      sendSyncState('mcp', label)

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

      const index = servers.value.findIndex(s => s.id === id);
      if (index !== -1) {
        servers.value[index].disabled = false;
      }

      sendSyncState('mcp', label)
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

      const index = servers.value.findIndex(s => s.id === id);
      if (index !== -1) {
        servers.value[index].disabled = true;
      }

      sendSyncState('mcp', label)
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

      sendSyncState('mcp', label)
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
    const serverToUpdate = { ...servers.value[serverIndex] };
    // With the change in fetchMcpServers, serverToUpdate.config.disabled_tools is guaranteed to be an array.

    try {
      const toolName = tool.name;
      const isDisabled = serverToUpdate.config.disabled_tools.includes(toolName);

      if (isDisabled) {
        // Tool is currently disabled, so enable it (remove from disabled_tools)
        serverToUpdate.config.disabled_tools = serverToUpdate.config.disabled_tools.filter(name => name !== toolName);
        console.debug(`Enabling tool "${toolName}" for server ID ${serverId}`);
      } else {
        // Tool is currently enabled, so disable it (add to disabled_tools)
        serverToUpdate.config.disabled_tools.push(toolName);
        console.debug(`Disabling tool "${toolName}" for server ID ${serverId}`);
      }

      // Call the backend to update the server configuration
      // updateMcpServer will handle the invoke call and refresh the servers list.
      const server = await invoke('update_mcp_tool_status', { id: serverId, toolName: toolName, disabled: !isDisabled });
      if (server) {
        // Update the servers list with the modified server object
        // Ensure the backend response's disabled_tools is also an array.
        const backendConfig = server.config || {};
        const backendDisabledTools = Array.isArray(backendConfig.disabled_tools)
          ? backendConfig.disabled_tools
          : [];
        servers.value[serverIndex] = {
          ...server,
          config: { ...backendConfig, disabled_tools: backendDisabledTools },
        };
      }

      sendSyncState('mcp', label)
      return server;
    } catch (err) {
      console.error(`Failed to toggle tool "${toolName}" state for server ID ${serverId} via backend:`, err);
      throw err; // Re-throw the error
    }
  }

  fetchMcpServers();

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
    toggleDisableTool
  };
});