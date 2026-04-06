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
  const waitReason = ref(null);
  const hasLiveSession = ref(false);
  const error = ref(null);
  const notification = ref({
    message: '',
    category: 'info',
    timestamp: 0
  });
  const autoApprovedTools = ref([]);
  const shellPolicy = ref([]);
  const toolStreams = ref(new Map()); // tool_id -> string[] (max 100 lines)

  const currentWorkflow = computed(() => {
    return workflows.value.find(w => w.id === currentWorkflowId.value);
  });

  const runningLikeStates = ['thinking', 'executing', 'auditing', 'running'];
  const waitingLikeStates = ['paused', 'awaiting_user', 'awaiting_approval', 'awaiting_auto_approval'];
  const approvalWaitingStates = ['awaiting_approval', 'awaiting_auto_approval'];

  const findLatestPendingApprovalMessage = (list = []) => {
    const finalizedIds = new Set();
    for (let i = list.length - 1; i >= 0; i--) {
      const msg = list[i];
      if (msg?.role !== 'tool') continue;
      const meta = msg.metadata || {};
      const toolCallId = meta.tool_call_id;
      const approvalStatus = meta.approval_status;
      if (!toolCallId) continue;
      if (approvalStatus === 'approved' || approvalStatus === 'rejected') {
        finalizedIds.add(toolCallId);
        continue;
      }
      if (approvalStatus === 'pending' && !finalizedIds.has(toolCallId)) {
        return msg;
      }
    }
    return null;
  };

  const isActivelyRunning = computed(() => {
    if (!hasLiveSession.value) return false;
    const status = currentWorkflow.value?.status?.toLowerCase() || '';
    return runningLikeStates.includes(status);
  });

  const isLiveWaiting = computed(() => {
    if (!hasLiveSession.value) return false;
    const status = currentWorkflow.value?.status?.toLowerCase() || '';
    return waitingLikeStates.includes(status);
  });

  const isOrphanWaiting = computed(() => {
    if (hasLiveSession.value) return false;
    const status = currentWorkflow.value?.status?.toLowerCase() || '';
    return waitingLikeStates.includes(status);
  });

  const isWaiting = computed(() => {
    const status = currentWorkflow.value?.status?.toLowerCase() || '';
    return waitingLikeStates.includes(status);
  });

  const canStop = computed(() => {
    return hasLiveSession.value && (isActivelyRunning.value || isLiveWaiting.value);
  });

  const canContinue = computed(() => {
    if (!hasLiveSession.value) return false;
    const status = currentWorkflow.value?.status?.toLowerCase() || '';
    return status === 'paused' && waitReason.value === 'confirmation';
  });

  const pendingApprovalMessage = computed(() => {
    return findLatestPendingApprovalMessage(messages.value);
  });

  const canApprovePending = computed(() => {
    const status = currentWorkflow.value?.status?.toLowerCase() || '';
    const isApprovalWaiting = waitReason.value === 'approval' || approvalWaitingStates.includes(status);
    if (!isApprovalWaiting) return false;
    return !!pendingApprovalMessage.value;
  });

  const canApprovePlan = computed(() => {
    const status = currentWorkflow.value?.status?.toLowerCase() || '';
    const isApprovalWaiting = waitReason.value === 'approval' || approvalWaitingStates.includes(status);
    if (!isApprovalWaiting) return false;
    return !canApprovePending.value;
  });

  /**
   * Extract allowed shell commands from shellPolicy
   * Returns array of { pattern, description } for commands with decision "allow"
   */
  const allowedShellCommands = computed(() => {
    if (!shellPolicy.value || !Array.isArray(shellPolicy.value)) return [];
    return shellPolicy.value
      .filter(item => item.decision === 'allow' && item.pattern)
      .map(item => ({
        pattern: item.pattern,
        description: item.description || ''
      }));
  });

  const setShellPolicy = (policy) => {
    shellPolicy.value = policy || [];
  };

  const setNotification = (message, category = 'info') => {
    notification.value = {
      message,
      category,
      timestamp: Date.now()
    };
  };

  const setAutoApprovedTools = (tools) => {
    autoApprovedTools.value = tools;
  };

  const removeAutoApprovedTool = (tool) => {
    const index = autoApprovedTools.value.indexOf(tool);
    if (index > -1) {
      autoApprovedTools.value.splice(index, 1);
    }
  };

  const removeShellPolicyItem = (pattern) => {
    const index = shellPolicy.value.findIndex(item => item.pattern === pattern);
    if (index > -1) {
      shellPolicy.value.splice(index, 1);
    }
  };

  /**
   * Append a line to tool stream output, keeping max 100 lines
   */
  const appendToolStream = (toolId, line) => {
    if (!toolStreams.value.has(toolId)) {
      toolStreams.value.set(toolId, [])
    }
    const lines = toolStreams.value.get(toolId)
    lines.push(line)
    // Keep only latest 100 lines
    if (lines.length > 100) {
      lines.splice(0, lines.length - 100)
    }
  }

  /**
   * Clear tool stream for a specific tool
   */
  const clearToolStream = (toolId) => {
    toolStreams.value.delete(toolId)
  }

  /**
   * Get tool stream lines for a specific tool
   */
  const getToolStream = (toolId) => {
    return toolStreams.value.get(toolId) || []
  }

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
   * Parse agentConfig from JSON string to object
   * agentConfig is stored as JSON string in DB with camelCase field names
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

    // Extract allowedPaths and shellPolicy from agentConfig
    w.allowedPaths = w.agentConfig.allowedPaths || [];
    w.shellPolicy = w.agentConfig.shellPolicy || [];
    // w.autoApprove = w.agentConfig.autoApprove || [];
    // w.approvalLevel = w.agentConfig.approvalLevel || 'default';
    // w.finalAudit = w.agentConfig.finalAudit || false;
    // w.maxContexts = w.agentConfig.maxContexts || null;
    // w.models = w.agentConfig.models || {};

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

      // Parse workflow data (agentConfig, allowedPaths, shellPolicy)
      _parseWorkflowData(snapshot.workflow);

      // Set shell policy from parsed workflow data
      setShellPolicy(snapshot.workflow.shellPolicy || []);

      const status = snapshot.workflow.status?.toLowerCase() || 'pending';
      hasLiveSession.value = snapshot.hasLiveSession || false;
      // isRunning means "actively processing with a live runtime session"
      isRunning.value = [
        'thinking',
        'executing',
        'auditing',
        'running'
      ].includes(status) && hasLiveSession.value;

      waitReason.value = snapshot.workflow.waitReason || null;

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

      // Fetch auto-approved tools for this workflow
      try {
        const tools = await invokeWrapper('get_auto_approved_tools', { sessionId: workflowId })
        if (tools && Array.isArray(tools)) {
          autoApprovedTools.value = tools
        } else {
          autoApprovedTools.value = []
        }
      } catch (e) {
        console.log('Could not fetch auto-approved tools:', e)
        autoApprovedTools.value = []
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

  const updateWorkflowStatus = async (workflowId, status, waitReasonValue = null) => {
    error.value = null;
    try {
      // Always update waitReason, even when null (to clear previous waiting state)
      waitReason.value = waitReasonValue;
      
      // Local status update for all engine states
      const statusLower = status.toLowerCase();
      const localUpdateStates = [
        'thinking', 'executing', 'auditing',
        'paused', 'awaiting_user', 'awaiting_approval', 'awaiting_auto_approval',
        'completed', 'error', 'cancelled', 'failed'
      ];
      
      if (localUpdateStates.includes(statusLower)) {
        const workflowIndex = workflows.value.findIndex(w => w.id === workflowId);
        if (workflowIndex !== -1) {
          workflows.value[workflowIndex].status = status;
          workflows.value[workflowIndex].waitReason = waitReasonValue;
        }

        // isRunning means "actively processing with a live runtime session"
        isRunning.value = [
          'thinking',
          'executing',
          'auditing',
          'running'
        ].includes(statusLower) && hasLiveSession.value;
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
    waitReason.value = null;
    hasLiveSession.value = false;
    autoApprovedTools.value = [];
    shellPolicy.value = [];
  };

  return {
    workflows,
    currentWorkflowId,
    messages,
    todoList,
    messageQueue,
    isRunning,
    waitReason,
    hasLiveSession,
    error,
    notification,
    autoApprovedTools,
    shellPolicy,
    allowedShellCommands,
    currentWorkflow,
    isActivelyRunning,
    isLiveWaiting,
    isOrphanWaiting,
    isWaiting,
    canStop,
    canContinue,
    pendingApprovalMessage,
    canApprovePending,
    canApprovePlan,
    setNotification,
    setAutoApprovedTools,
    removeAutoApprovedTool,
    setShellPolicy,
    removeShellPolicyItem,
    appendToolStream,
    clearToolStream,
    getToolStream,
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
