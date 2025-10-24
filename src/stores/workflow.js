import { FrontendAppError, invokeWrapper } from '@/libs/tauri';
import { defineStore } from 'pinia';
import { computed, ref } from 'vue';

export const useWorkflowStore = defineStore('workflow', () => {
  const workflows = ref([]);
  const currentWorkflowId = ref(null);
  const messages = ref([]);
  const messageQueue = ref([]);
  const isRunning = ref(false);
  const error = ref(null);

  const currentWorkflow = computed(() => {
    return workflows.value.find(w => w.id === currentWorkflowId.value);
  });

  const _handleError = async (err) => {
    if (err instanceof FrontendAppError) {
      error.value = err.toFormattedString();
      console.error('Workflow Store Error:', error.value, err.originalError);
    } else {
      error.value = err.message || String(err);
      console.error('Workflow Store Error:', error.value);
    }
    throw err; // Re-throw for component-level handling if needed
  };

  const loadWorkflows = async () => {
    error.value = null;
    try {
      const result = await invokeWrapper('list_workflows');
      workflows.value = result || [];
    } catch (err) {
      await _handleError(err);
    }
  };

  const selectWorkflow = async (workflowId) => {
    currentWorkflowId.value = workflowId;
    error.value = null;
    try {
      const snapshot = await invokeWrapper('get_workflow_snapshot', { workflowId });
      messages.value = snapshot.messages || [];
    } catch (err) {
      await _handleError(err);
      messages.value = [];
    }
  };

  const createWorkflow = async (userQuery, agentId) => {
    error.value = null;
    try {
      const newWorkflow = await invokeWrapper('create_workflow', { userQuery, agentId });
      workflows.value.unshift(newWorkflow);
      await selectWorkflow(newWorkflow.id);
      return newWorkflow;
    } catch (err) {
      await _handleError(err);
    }
  };

  const addMessageToQueue = (message) => {
    messageQueue.value.push(message);
  };

  return {
    workflows,
    currentWorkflowId,
    messages,
    messageQueue,
    isRunning,
    error,
    currentWorkflow,
    loadWorkflows,
    selectWorkflow,
    createWorkflow,
    addMessageToQueue,
  };
});
