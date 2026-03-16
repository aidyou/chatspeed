import { FrontendAppError, invokeWrapper } from '@/libs/tauri';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { defineStore } from 'pinia';
import { ref } from 'vue';

import { sendSyncState } from '@/libs/sync';

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
 * @property {boolean} finalAudit - Whether the agent requires final audit.
 * @property {string} approvalLevel - Approval level (default, smart, full).
 */

const label = getCurrentWebviewWindow().label;

/**
 * Transforms agent data from the backend (snake_case, JSON strings)
 * to the frontend format (camelCase, objects).
 */
const _transformFromBackend = (backendAgent) => {
  if (!backendAgent) return null;

  // Default model config
  const defaultModel = { id: '', model: '', temperature: -0.1, contextSize: 128000, maxTokens: 0 };

  // models field is already an object (struct AgentModels), not a JSON string
  // Tauri IPC auto-serializes Rust structs to JS objects
  let models = {
    plan: { ...defaultModel },
    act: { ...defaultModel },
    vision: { ...defaultModel }
  };

  if (backendAgent.models) {
    // models is already an object { plan: {...}, act: {...}, vision: {...} }
    if (backendAgent.models.plan) {
      models.plan = { ...defaultModel, ...backendAgent.models.plan };
    }
    if (backendAgent.models.act) {
      models.act = { ...defaultModel, ...backendAgent.models.act };
    }
    if (backendAgent.models.vision) {
      models.vision = { ...defaultModel, ...backendAgent.models.vision };
    }
  }

  return {
    id: backendAgent.id,
    name: backendAgent.name,
    description: backendAgent.description,
    systemPrompt: backendAgent.system_prompt,
    planningPrompt: backendAgent.planning_prompt,

    // These are JSON strings, need to parse
    availableTools: backendAgent.available_tools ? JSON.parse(backendAgent.available_tools) : [],
    autoApprove: backendAgent.auto_approve ? JSON.parse(backendAgent.auto_approve) : [],

    // Models are already objects
    planModel: models.plan,
    actModel: models.act,
    visionModel: models.vision,

    // These are JSON strings, need to parse
    shellPolicy: backendAgent.shell_policy ? JSON.parse(backendAgent.shell_policy) : [],
    allowedPaths: backendAgent.allowed_paths ? JSON.parse(backendAgent.allowed_paths) : [],

    models: backendAgent.models || null,
    maxContexts: backendAgent.max_contexts || 128000,
    finalAudit: !!backendAgent.final_audit,
    approvalLevel: backendAgent.approval_level || 'default'
  };
};

/**
 * Transforms agent data from the frontend (camelCase, objects)
 * to the backend format (snake_case, JSON strings).
 */
const _transformToBackend = (frontendAgent) => {
  // Build models as an object (not JSON string) - backend expects struct AgentModels
  // When model is empty/not selected, pass null instead of object with empty id
  const buildModelConfig = (modelObj) => {
    if (modelObj?.id !== undefined && modelObj.id !== '' && modelObj.model) {
      return {
        id: typeof modelObj.id === 'string' ? parseInt(modelObj.id, 10) : modelObj.id,
        model: modelObj.model,
        temperature: modelObj.temperature ?? -0.1,
        contextSize: modelObj.contextSize ?? 128000,
        maxTokens: modelObj.maxTokens ?? 0
      };
    }
    return null;
  };

  const modelsObj = {
    plan: buildModelConfig(frontendAgent.planModel),
    act: buildModelConfig(frontendAgent.actModel),
    vision: buildModelConfig(frontendAgent.visionModel)
  };

  return {
    id: frontendAgent.id || '',
    name: frontendAgent.name.trim(),
    description: frontendAgent.description?.trim() || '',
    system_prompt: frontendAgent.systemPrompt.trim(),
    planning_prompt: frontendAgent.planningPrompt?.trim() || '',
    // JSON strings
    available_tools: JSON.stringify(frontendAgent.availableTools || []),
    auto_approve: JSON.stringify(frontendAgent.autoApprove || []),
    shell_policy: JSON.stringify(frontendAgent.shellPolicy || []),
    allowed_paths: JSON.stringify(frontendAgent.allowedPaths || []),
    // Struct object
    models: modelsObj,
    max_contexts: frontendAgent.maxContexts,
    final_audit: !!frontendAgent.finalAudit,
    approval_level: frontendAgent.approvalLevel || 'default'
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
      sendSyncState('agent', label);
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
      sendSyncState('agent', label);
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
      sendSyncState('agent', label);
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
    updateAgentStore: fetchAgents,
    fetchAvailableTools,
    getAgent,
    saveAgent,
    deleteAgent,
    copyAgent,
    updateAgentOrder,
  };
});
