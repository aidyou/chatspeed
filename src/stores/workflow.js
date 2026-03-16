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
  const notification = ref({
    message: '',
    category: 'info',
    timestamp: 0
  });

  const currentWorkflow = computed(() => {
    return workflows.value.find(w => w.id === currentWorkflowId.value);
  });

  const setNotification = (message, category = 'info') => {
    notification.value = {
      message,
      category,
      timestamp: Date.now()
    };
  };

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

  /**
   * Parse agentConfig and extract allowedPaths from it
   * agentConfig is stored as JSON string in DB, with structure:
   * {
   *   "allowed_paths": [...],
   *   "models": {...},
   *   "shell_policy": [...],
   *   "approval_level": "...",
   *   "final_audit": true/false
   * }
   *
   * This function normalizes the internal field names from snake_case to camelCase
   */
  const _parseWorkflowData = (w) => {
    // Initialize defaults
    if (!w.agentConfig) {
      w.agentConfig = {};
    } else if (typeof w.agentConfig === 'string') {
      try {
        w.agentConfig = JSON.parse(w.agentConfig);
      } catch (e) {
        console.error('Failed to parse agentConfig for workflow', w.id, e);
        w.agentConfig = {};
      }
    }

    // Normalize agentConfig internal fields from snake_case to camelCase
    if (w.agentConfig && typeof w.agentConfig === 'object') {
      const config = w.agentConfig;

      // allowed_paths -> allowedPaths
      if (config.allowed_paths !== undefined) {
        config.allowedPaths = config.allowed_paths;
        delete config.allowed_paths;
      }

      // shell_policy -> shellPolicy
      if (config.shell_policy !== undefined) {
        config.shellPolicy = config.shell_policy;
        delete config.shell_policy;
      }

      // approval_level -> approvalLevel
      if (config.approval_level !== undefined) {
        config.approvalLevel = config.approval_level;
        delete config.approval_level;
      }

      // final_audit -> finalAudit
      if (config.final_audit !== undefined) {
        config.finalAudit = config.final_audit;
        delete config.final_audit;
      }
    }

    // Extract allowedPaths from agentConfig (now in camelCase)
    w.allowedPaths = w.agentConfig.allowedPaths || [];

    return w;
  };

  const loadWorkflows = async () => {
    error.value = null;
    try {
      const result = await invokeWrapper('list_workflows');
      workflows.value = (result || []).map(w => _parseWorkflowData(w));
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

      // Parse workflow data (agentConfig, allowedPaths)
      _parseWorkflowData(snapshot.workflow);

      // Parse metadata for all messages in snapshot
      const parsedMessages = (snapshot.messages || []).map(m => {
        if (m.metadata && typeof m.metadata === 'string') {
          try {
            m.metadata = JSON.parse(m.metadata);
          } catch (e) {
            console.error('Failed to parse snapshot message metadata:', e);
          }
        }
        // Normalize is_error to isError
        if (m.is_error !== undefined) {
          m.isError = m.is_error;
        }
        return m;
      });

      messages.value = parsedMessages;

      // Initialize todo list from workflow snapshot
      if (snapshot.workflow.todoList) {
        try {
          const todos = JSON.parse(snapshot.workflow.todoList);
          todoList.value = todos;
        } catch (e) {
          console.error('Failed to parse todo list from workflow:', e);
          todoList.value = [];
        }
      } else {
        todoList.value = [];
      }

      // Update workflow in the list with the parsed data
      const workflowIndex = workflows.value.findIndex(w => w.id === workflowId);
      console.log('workflowStore: workflowIndex for update:', workflowIndex);
      if (workflowIndex !== -1) {
        workflows.value[workflowIndex] = {
          ...workflows.value[workflowIndex],
          ...snapshot.workflow
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
      // No ID passed here, backend will generate TSID
      const newWorkflowId = await invokeWrapper('create_workflow', {
        workflow: {
          userQuery,
          agentId,
          allowedPaths: allowedPaths
        }
      });

      // Reload workflows to get the fully populated object from DB
      await loadWorkflows();

      // Select the new workflow using the TSID returned by backend
      await selectWorkflow(newWorkflowId);

      return newWorkflowId;
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
    // Note: metadata is already an object from Rust (serde_json::Value)
    // No need to parse, just ensure it's not null/undefined
    if (!message.metadata) {
      message.metadata = {};
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
        allowedPaths: allowedPaths
      });
      const workflowIndex = workflows.value.findIndex(w => w.id === workflowId);
      if (workflowIndex !== -1) {
        workflows.value[workflowIndex].allowedPaths = allowedPaths;
      }
    } catch (err) {
      await _handleError(err);
    }
  };

  const updateWorkflowFinalAudit = async (workflowId, finalAudit) => {
    error.value = null;
    try {
      await invokeWrapper('update_workflow_final_audit', {
        sessionId: workflowId,
        finalAudit: finalAudit
      });
      const workflowIndex = workflows.value.findIndex(w => w.id === workflowId);
      if (workflowIndex !== -1) {
        workflows.value[workflowIndex].finalAudit = finalAudit;
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
    notification,
    currentWorkflow,
    setNotification,
    setTodoList,
    loadWorkflows,
    selectWorkflow,
    createWorkflow,
    addMessage,
    addMessageToQueue,
    setRunning,
    updateWorkflowStatus,
    updateWorkflowAllowedPaths,
    updateWorkflowFinalAudit,
    loadMessages,
    clearCurrentWorkflow,
  };
});
