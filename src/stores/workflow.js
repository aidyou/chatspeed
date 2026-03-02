import { FrontendAppError, invokeWrapper } from '@/libs/tauri';
import { defineStore } from 'pinia';
import { computed, ref } from 'vue';

export const useWorkflowStore = defineStore('workflow', () => {
  const workflows = ref([]);
  const currentWorkflowId = ref(null);
  const messages = ref([]);
  const todoList = ref([]);
  const messageQueue = ref([]);
  const isRunning = ref(false);
  const error = ref(null);

  const currentWorkflow = computed(() => {
    return workflows.value.find(w => w.id === currentWorkflowId.value);
  });

  const setTodoList = (todos) => {
    todoList.value = todos;
  };

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
      workflows.value = (result || []).map(w => {
        if (w.allowedPaths && typeof w.allowedPaths === 'string') {
          try {
            w.allowedPaths = JSON.parse(w.allowedPaths);
          } catch (e) {
            console.error('Failed to parse allowedPaths for workflow', w.id, e);
            w.allowedPaths = [];
          }
        } else if (!w.allowedPaths) {
          w.allowedPaths = [];
        }
        return w;
      });
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
      
      // Parse metadata for all messages in snapshot
      const parsedMessages = (snapshot.messages || []).map(m => {
        if (m.metadata && typeof m.metadata === 'string') {
          try {
            m.metadata = JSON.parse(m.metadata);
          } catch (e) {
            console.error('Failed to parse snapshot message metadata:', e);
          }
        }
        return m;
      });
      
      messages.value = parsedMessages;

      // Initialize todo list from workflow snapshot
      if (snapshot.workflow.todoList) {
        try {
          const todos = JSON.parse(snapshot.workflow.todoList);
          todoList.value = todos;
          
          // Still sync with todo manager for tool consistency
          const { setTodoListForWorkflow } = await import('@/pkg/workflow/tools/todoList');
          setTodoListForWorkflow(workflowId, todos);
        } catch (e) {
          console.error('Failed to parse todo list from workflow:', e);
          todoList.value = [];
        }
      } else {
        todoList.value = [];
      }

      // Initialize allowedPaths in the workflow object in the list
      const workflowIndex = workflows.value.findIndex(w => w.id === workflowId);
      console.log('workflowStore: workflowIndex for update:', workflowIndex);
      if (workflowIndex !== -1) {
        let paths = [];
        if (snapshot.workflow.allowedPaths) {
          try {
            paths = typeof snapshot.workflow.allowedPaths === 'string' 
              ? JSON.parse(snapshot.workflow.allowedPaths) 
              : snapshot.workflow.allowedPaths;
          } catch (e) {
            console.error('Failed to parse allowedPaths from snapshot:', e);
          }
        }
        console.log('workflowStore: setting allowedPaths to:', paths);
        // Force update the object in the list to trigger reactivity
        workflows.value[workflowIndex] = {
          ...workflows.value[workflowIndex],
          ...snapshot.workflow,
          allowedPaths: Array.isArray(paths) ? paths : []
        };
      }
    } catch (err) {
      await _handleError(err);
      messages.value = [];
      todoList.value = [];
    }
  };

  const createWorkflow = async (userQuery, agentId, allowedPaths = []) => {
    error.value = null;
    try {
      const id = `session_${Date.now()}`;
      const newWorkflow = await invokeWrapper('create_workflow', {
        workflow: {
          id,
          userQuery,
          agentId,
          status: 'pending',
          allowedPaths: JSON.stringify(allowedPaths),
          createdAt: new Date().toISOString(),
          updatedAt: new Date().toISOString()
        }
      });
      // Backend returns the ID string, we should fetch or construct the object
      const workflowObj = {
        id: typeof newWorkflow === 'string' ? newWorkflow : id,
        userQuery,
        agentId,
        status: 'pending',
        allowedPaths
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
    // Ensure metadata is an object
    if (message.metadata && typeof message.metadata === 'string') {
      try {
        message.metadata = JSON.parse(message.metadata);
      } catch (e) {
        console.error('Failed to parse message metadata:', e);
      }
    }

    const index = messages.value.findIndex(m =>
      (message.id && m.id === message.id) ||
      (m.stepIndex === message.stepIndex && m.role === message.role && m.stepType === message.stepType && m.message === message.message)
    );

    if (index !== -1) {
      // Update existing message
      messages.value[index] = { ...messages.value[index], ...message };
    } else {
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

  const updateWorkflowAllowedPaths = async (workflowId, allowedPaths) => {
    error.value = null;
    try {
      await invokeWrapper('update_workflow_allowed_paths', {
        sessionId: workflowId,
        allowedPaths: JSON.stringify(allowedPaths)
      });
      const workflowIndex = workflows.value.findIndex(w => w.id === workflowId);
      if (workflowIndex !== -1) {
        workflows.value[workflowIndex].allowedPaths = allowedPaths;
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
    todoList.value = [];
    isRunning.value = false;
  };

  return {
    workflows,
    currentWorkflowId,
    messages,
    todoList,
    messageQueue,
    isRunning,
    error,
    currentWorkflow,
    setTodoList,
    loadWorkflows,
    selectWorkflow,
    createWorkflow,
    addMessage,
    addMessageToQueue,
    setRunning,
    updateWorkflowStatus,
    updateWorkflowAllowedPaths,
    loadMessages,
    deleteMessage,
    clearCurrentWorkflow,
  };
});
