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

      // Initialize todo manager with the workflow's todo list
      if (snapshot.workflow.todoList) {
        try {
          const todoList = JSON.parse(snapshot.workflow.todoList);
          // Import the todo list into the todo manager
          const { setTodoListForWorkflow } = await import('@/pkg/workflow/tools/todoList');
          setTodoListForWorkflow(workflowId, todoList);
        } catch (e) {
          console.error('Failed to parse todo list from workflow:', e);
        }
      }
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

  const setRunning = (running) => {
    isRunning.value = running;
  };

  const updateWorkflowStatus = async (workflowId, status) => {
    error.value = null;
    try {
      await invokeWrapper('update_workflow_status', { workflowId, status });

      // Update local workflow if it's the current one
      const workflowIndex = workflows.value.findIndex(w => w.id === workflowId);
      if (workflowIndex !== -1) {
        workflows.value[workflowIndex].status = status;
      }

      // Update running state based on status
      if (status === 'running') {
        isRunning.value = true;
      } else if (status === 'completed' || status === 'error' || status === 'paused') {
        isRunning.value = false;
      }
    } catch (err) {
      await _handleError(err);
    }
  };

  const clearCurrentWorkflow = () => {
    currentWorkflowId.value = null;
    messages.value = [];
    isRunning.value = false;
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
    setRunning,
    updateWorkflowStatus,
    clearCurrentWorkflow,
  };
});
