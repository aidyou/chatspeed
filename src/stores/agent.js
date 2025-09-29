import { invoke } from '@tauri-apps/api/core';
import { defineStore } from 'pinia';
import { ref } from 'vue';

/**
 * @typedef {Object} Agent
 * @property {string} id - The unique identifier of the agent.
 * @property {string} name - The name of the agent.
 * @property {string} description - The description of the agent.
 * @property {string} systemPrompt - The system prompt for the agent.
 * @property {string[]} availableTools - A list of tool IDs available to the agent.
 * @property {string[]} autoApprove - A list of tool IDs that are auto-approved.
 * @property {string} planModel - The model used for planning.
 * @property {string} actModel - The model used for acting.
 * @property {string} visionModel - The model used for vision tasks.
 * @property {number} maxContexts - The maximum context length.
 */

/**
 * @typedef {Object} Tool
 * @property {string} id - The unique identifier of the tool.
 * @property {string} name - The name of the tool.
 * @property {string} description - The description of the tool.
 * @property {string} category - The category of the tool (e.g., "Web", "MCP").
 */

/**
 * Transforms agent data from the backend (snake_case, JSON strings)
 * to the frontend format (camelCase, objects).
 * @param {Object} backendAgent - The agent object from the backend.
 * @returns {Agent} The transformed agent object for the frontend.
 */
const _transformFromBackend = (backendAgent) => {
  if (!backendAgent) return null;

  const parseModel = (modelStr) => {
    try {
      if (modelStr && typeof modelStr === 'string') return JSON.parse(modelStr);
    } catch (e) {
      console.error('Failed to parse model string:', modelStr, e);
    }
    return { id: '', model: '' };
  };

  return {
    id: backendAgent.id,
    name: backendAgent.name,
    description: backendAgent.description,
    systemPrompt: backendAgent.system_prompt,
    agentType: backendAgent.agent_type || 'autonomous',
    planningPrompt: backendAgent.planning_prompt || '',
    availableTools: backendAgent.available_tools ? JSON.parse(backendAgent.available_tools) : [],
    autoApprove: backendAgent.auto_approve ? JSON.parse(backendAgent.auto_approve) : [],
    planModel: parseModel(backendAgent.plan_model),
    actModel: parseModel(backendAgent.act_model),
    visionModel: parseModel(backendAgent.vision_model),
    maxContexts: backendAgent.max_contexts || 128000
  };
};

/**
 * Transforms agent data from the frontend (camelCase, objects)
 * to the backend format (snake_case, JSON strings).
 * @param {Agent} frontendAgent - The agent object from the frontend.
 * @returns {Object} The transformed agent payload for the backend.
 */
const _transformToBackend = (frontendAgent) => {
  const stringifyModel = (modelObj) => {
    if (modelObj && modelObj.id && modelObj.model) {
      return JSON.stringify(modelObj);
    }
    return '';
  };

  return {
    id: frontendAgent.id,
    name: frontendAgent.name.trim(),
    description: frontendAgent.description?.trim() || '',
    system_prompt: frontendAgent.systemPrompt.trim(),
    agent_type: frontendAgent.agentType || 'autonomous',
    planning_prompt: frontendAgent.planningPrompt?.trim() || '',
    available_tools: JSON.stringify(frontendAgent.availableTools || []),
    auto_approve: JSON.stringify(frontendAgent.autoApprove || []),
    plan_model: stringifyModel(frontendAgent.planModel),
    act_model: stringifyModel(frontendAgent.actModel),
    vision_model: stringifyModel(frontendAgent.visionModel),
    max_contexts: frontendAgent.maxContexts
  };
};


export const useAgentStore = defineStore('agent', () => {
  /** @type {import('vue').Ref<Agent[]>} */
  const agents = ref([]);
  /** @type {import('vue').Ref<Tool[]>} */
  const availableTools = ref([]);

  const loading = ref(false);
  const error = ref(null);

  const _handleError = (err, message = 'Agent Store Error') => {
    error.value = err.message || String(err);
    loading.value = false;
    console.error(`${message}:`, error.value);
    throw err;
  };

  /**
   * Fetches all agents from the backend.
   */
  const fetchAgents = async () => {
    loading.value = true;
    error.value = null;
    try {
      const result = await invoke('get_all_agents');
      agents.value = (result || []).map(_transformFromBackend);
    } catch (err) {
      _handleError(err, 'Failed to fetch agents');
    } finally {
      loading.value = false;
    }
  };

  /**
   * Fetches all available tools from the backend.
   */
  const fetchAvailableTools = async () => {
    loading.value = true;
    error.value = null;
    try {
      const result = await invoke('get_available_tools');
      availableTools.value = result || [];
    } catch (err) {
      _handleError(err, 'Failed to fetch available tools');
    } finally {
      loading.value = false;
    }
  };

  /**
   * Fetches a single agent by its ID.
   * @param {string} id - The ID of the agent to fetch.
   * @returns {Promise<Agent|null>} The agent data.
   */
  const getAgent = async (id) => {
    loading.value = true;
    error.value = null;
    try {
      const agentData = await invoke('get_agent', { id });
      return _transformFromBackend(agentData);
    } catch (err) {
      _handleError(err, `Failed to fetch agent ${id}`);
    } finally {
      loading.value = false;
    }
  };

  /**
   * Saves an agent (creates a new one or updates an existing one).
   * @param {Agent} payload - The agent data from the form.
   * @returns {Promise<void>}
   */
  const saveAgent = async (payload) => {
    loading.value = true;
    error.value = null;
    try {
      const agentPayload = _transformToBackend(payload);
      const command = agentPayload.id ? 'update_agent' : 'add_agent';
      await invoke(command, { agentPayload });
      await fetchAgents(); // Refresh the list
    } catch (err) {
      _handleError(err, 'Failed to save agent');
    } finally {
      loading.value = false;
    }
  };

  /**
   * Deletes an agent by its ID.
   * @param {string} id - The ID of the agent to delete.
   */
  const deleteAgent = async (id) => {
    loading.value = true;
    error.value = null;
    try {
      await invoke('delete_agent', { id });
      const index = agents.value.findIndex(a => a.id === id);
      if (index !== -1) {
        agents.value.splice(index, 1);
      }
    } catch (err) {
      _handleError(err, `Failed to delete agent ${id}`);
    } finally {
      loading.value = false;
    }
  };

  /**
   * Fetches an agent and prepares it for copying.
   * @param {string} id - The ID of the agent to copy.
   * @returns {Promise<Agent>} A new agent object ready for the edit form.
   */
  const copyAgent = async (id) => {
    const agentToCopy = await getAgent(id);
    if (!agentToCopy) {
      throw new Error('Agent to copy not found');
    }
    return {
      ...agentToCopy,
      id: null, // Remove ID to indicate it's a new agent
      name: `${agentToCopy.name}-Copy`,
    };
  };

  /**
   * Updates the order of agents.
   * NOTE: Backend command 'update_agent_order' is assumed to exist.
   * @param {Agent[]} orderedAgents - The array of agents in the new order.
   */
  const updateAgentOrder = async (orderedAgents) => {
    loading.value = true;
    error.value = null;
    try {
      const agentIds = orderedAgents.map(a => a.id);
      await invoke('update_agent_order', { agentIds });
      agents.value = [...orderedAgents]; // Update local state to reflect new order
    } catch (err) {
      _handleError(err, 'Failed to update agent order');
    } finally {
      loading.value = false;
    }
  };

  // Initial data fetch
  fetchAgents();
  fetchAvailableTools();

  return {
    agents,
    availableTools,
    loading,
    error,
    fetchAgents,
    fetchAvailableTools,
    getAgent,
    saveAgent,
    deleteAgent,
    copyAgent,
    updateAgentOrder,
  };
});
