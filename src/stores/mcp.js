import { defineStore } from 'pinia';
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';

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
      servers.value = fetchedServers.map(s => ({ ...s, enabled: !s.disabled, expanded: false, loading: false }));

      console.log(servers.value)
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
      await fetchMcpServers(); // Refresh the list
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
      await fetchMcpServers(); // Refresh the list
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
      await fetchMcpServers(); // Refresh the list
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
      await fetchMcpServers();
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
      await fetchMcpServers();
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
      await fetchMcpServers();
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
    } catch (err) {
      await _handleError(err);
    } finally {
      loading.value = false;
    }
  };

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
  };
});