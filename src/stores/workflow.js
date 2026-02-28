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
    console.log('workflowStore: selecting workflow', workflowId);
    currentWorkflowId.value = workflowId;
    error.value = null;
    try {
      const snapshot = await invokeWrapper('get_workflow_snapshot', { sessionId: workflowId });
      console.log('workflowStore: snapshot loaded', snapshot);
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
      const id = `session_${Date.now()}`;
      const newWorkflow = await invokeWrapper('create_workflow', {
        workflow: {
          id,
          userQuery,
          agentId,
          status: 'pending',
          createdAt: new Date().toISOString(),
          updatedAt: new Date().toISOString()
        }
      });
      // Backend returns the ID string, we should fetch or construct the object
      const workflowObj = {
        id: typeof newWorkflow === 'string' ? newWorkflow : id,
        userQuery,
        agentId,
        status: 'pending'
      };
      workflows.value.unshift(workflowObj);
      await selectWorkflow(workflowObj.id);
      return workflowObj;
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

  const addMessage = (message) => {
    // Check if message already exists by id (if available) or metadata
    const exists = messages.value.some(m =>
      (message.id && m.id === message.id) ||
      (m.stepIndex === message.stepIndex && m.role === message.role && m.stepType === message.stepType && m.message === message.message)
    );
    if (!exists) {
      messages.value.push(message);
    }
  };

  const updateWorkflowStatus = async (workflowId, status) => {
    error.value = null;
    try {
      // Avoid database update if it's an internal engine state transition that doesn't need persistence
      if (['thinking', 'executing', 'paused', 'completed', 'error'].includes(status.toLowerCase())) {
        const workflowIndex = workflows.value.findIndex(w => w.id === workflowId);
        if (workflowIndex !== -1) {
          workflows.value[workflowIndex].status = status;
        }

        // Update running state based on status
        const s = status.toLowerCase();
        if (s === 'thinking' || s === 'executing' || s === 'running') {
          isRunning.value = true;
        } else {
          isRunning.value = false;
        }
      } else {
        await invokeWrapper('update_workflow_status', { sessionId: workflowId, status });
      }
    } catch (err) {
      await _handleError(err);
    }
  };

  const loadMessages = async (workflowId) => {
    console.log('workflowStore: loading messages for', workflowId);
    error.value = null;
    try {
      const snapshot = await invokeWrapper('get_workflow_snapshot', { sessionId: workflowId });
      messages.value = snapshot.messages || [];
    } catch (err) {
      await _handleError(err);
    }
  };

  const deleteMessage = async (workflowId, messageId) => {
    error.value = null;
    try {
      await invokeWrapper('delete_message', { id: messageId });
      // The backend uses 'delete_message' command which takes 'id'
      // After deletion, we could either filter locally or reload
      messages.value = messages.value.filter(m => m.id !== messageId);
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
    addMessage,
    addMessageToQueue,
    setRunning,
    updateWorkflowStatus,
    loadMessages,
    deleteMessage,
    clearCurrentWorkflow,
  };
});
