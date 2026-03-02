import { FrontendAppError, invokeWrapper } from '@/libs/tauri';
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
 * @property {Object} planModel - The model used for planning.
 * @property {Object} actModel - The model used for acting.
 * @property {Object} visionModel - The model used for vision tasks.
 * @property {Object} codingModel - The model used for coding tasks.
 * @property {Object} copywritingModel - The model used for writing tasks.
 * @property {Object} browsingModel - The model used for browsing tasks.
 * @property {string} models - Unified JSON string for all models.
 * @property {number} maxContexts - The maximum context length.
 */

/**
 * @typedef {Object} Tool
 * @property {string} id - The unique identifier of the tool.
 * @property {string} name - The name of the tool.
 * @property {string} description - The description of the tool.
 * @property {string} category - The category of the tool (e.g., "Web", "FS", "System").
 */

/**
 * Transforms agent data from the backend (snake_case, JSON strings)
 * to the frontend format (camelCase, objects).
 */
const _transformFromBackend = (backendAgent) => {
  if (!backendAgent) return null;

  const parseModel = (modelStr) => {
    try {
      if (modelStr && typeof modelStr === 'string') return JSON.parse(modelStr);
      if (modelStr && typeof modelStr === 'object') return modelStr;
    } catch (e) {
      console.error('Failed to parse model string:', modelStr, e);
    }
    return { id: '', model: '' };
  };

  let models = {
    plan: parseModel(backendAgent.plan_model),
    act: parseModel(backendAgent.act_model),
    vision: parseModel(backendAgent.vision_model),
    coding: { id: '', model: '' },
    copywriting: { id: '', model: '' },
    browsing: { id: '', model: '' }
  };

  if (backendAgent.models) {
    try {
      const unifiedModels = JSON.parse(backendAgent.models);
      models = { ...models, ...unifiedModels };
    } catch (e) {
      console.error('Failed to parse unified models JSON:', e);
    }
  }

  return {
    id: backendAgent.id,
    name: backendAgent.name,
    description: backendAgent.description || '',
    systemPrompt: backendAgent.system_prompt,
    agentType: backendAgent.agent_type || 'autonomous',
    planningPrompt: backendAgent.planning_prompt || '',
    availableTools: backendAgent.available_tools ? JSON.parse(backendAgent.available_tools) : [],
    autoApprove: backendAgent.auto_approve ? JSON.parse(backendAgent.auto_approve) : [],
    planModel: models.plan,
    actModel: models.act,
    visionModel: models.vision,
    codingModel: models.coding,
    copywritingModel: models.copywriting,
    browsingModel: models.browsing,
    shellPolicy: backendAgent.shell_policy ? JSON.parse(backendAgent.shell_policy) : [],
    allowedPaths: backendAgent.allowed_paths ? JSON.parse(backendAgent.allowed_paths) : [],
    models: backendAgent.models || '',
    maxContexts: backendAgent.max_contexts || 128000
  };
};

/**
 * Transforms agent data from the frontend (camelCase, objects)
 * to the backend format (snake_case, JSON strings).
 */
const _transformToBackend = (frontendAgent) => {
  const stringifyModel = (modelObj) => {
    if (modelObj?.id !== undefined && modelObj.model) {
      return JSON.stringify(modelObj);
    }
    return '';
  };

  // If models field isn't already a consolidated string, create it
  let modelsJson = frontendAgent.models;
  if (!modelsJson) {
    modelsJson = JSON.stringify({
      plan: frontendAgent.planModel,
      act: frontendAgent.actModel,
      vision: frontendAgent.visionModel,
      coding: frontendAgent.codingModel,
      copywriting: frontendAgent.copywritingModel,
      browsing: frontendAgent.browsingModel
    });
  }

  return {
    id: frontendAgent.id,
    name: frontendAgent.name.trim(),
    description: frontendAgent.description?.trim() || '',
    system_prompt: frontendAgent.systemPrompt.trim(),
    agent_type: frontendAgent.agentType || 'autonomous',
    planning_prompt: frontendAgent.planningPrompt?.trim() || '',
    available_tools: JSON.stringify(frontendAgent.availableTools || []),
    auto_approve: JSON.stringify(frontendAgent.autoApprove || []),
    shell_policy: JSON.stringify(frontendAgent.shellPolicy || []),
    allowed_paths: JSON.stringify(frontendAgent.allowedPaths || []),
    plan_model: stringifyModel(frontendAgent.planModel),
    act_model: stringifyModel(frontendAgent.actModel),
    vision_model: stringifyModel(frontendAgent.visionModel),
    models: modelsJson,
    max_contexts: frontendAgent.maxContexts
  };
};


export const useAgentStore = defineStore('agent', () => {
  const agents = ref([]);
  const availableTools = ref([]);
  const loading = ref(false);
  const error = ref(null);

  const _handleError = (err, message = 'Agent Store Error') => {
    loading.value = false;
    if (err instanceof FrontendAppError) {
      error.value = err.toFormattedString();
      console.error(`${message}:`, error.value, err.originalError);
    } else {
      error.value = err.message || String(err);
      console.error(`${message}:`, error.value);
    }
    throw err;
  };

  const fetchAgents = async () => {
    loading.value = true;
    error.value = null;
    try {
      const result = await invokeWrapper('get_all_agents');
      agents.value = (result || []).map(_transformFromBackend);
    } catch (err) {
      _handleError(err, 'Failed to fetch agents');
    } finally {
      loading.value = false;
    }
  };

  const fetchAvailableTools = async () => {
    loading.value = true;
    error.value = null;
    try {
      const result = await invokeWrapper('get_available_tools');
      // Each result item now includes {id, name, category}
      availableTools.value = result || [];
    } catch (err) {
      _handleError(err, 'Failed to fetch available tools');
    } finally {
      loading.value = false;
    }
  };

  const getAgent = async (id) => {
    loading.value = true;
    error.value = null;
    try {
      const agentData = await invokeWrapper('get_agent', { id });
      return _transformFromBackend(agentData);
    } catch (err) {
      _handleError(err, `Failed to fetch agent ${id}`);
    } finally {
      loading.value = false;
    }
  };

  const saveAgent = async (payload) => {
    loading.value = true;
    error.value = null;
    try {
      const agent = _transformToBackend(payload);
      const command = agent.id ? 'update_agent' : 'add_agent';
      // Corrected payload key to match Rust command 'agent' parameter
      await invokeWrapper(command, { agent });
      await fetchAgents();
    } catch (err) {
      _handleError(err, 'Failed to save agent');
    } finally {
      loading.value = false;
    }
  };

  const deleteAgent = async (id) => {
    loading.value = true;
    error.value = null;
    try {
      await invokeWrapper('delete_agent', { id });
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

  const copyAgent = async (id) => {
    const agentToCopy = await getAgent(id);
    if (!agentToCopy) {
      throw new Error('Agent to copy not found');
    }
    return {
      ...agentToCopy,
      id: null,
      name: `${agentToCopy.name}-Copy`,
    };
  };

  const updateAgentOrder = async (orderedAgents) => {
    loading.value = true;
    error.value = null;
    try {
      const agentIds = orderedAgents.map(a => a.id);
      await invokeWrapper('update_agent_order', { agentIds });
      agents.value = [...orderedAgents];
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
