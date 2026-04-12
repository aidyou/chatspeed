import { FrontendAppError, invokeWrapper } from '@/libs/tauri';
import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import {
  APPROVAL_WAITING_STATUSES,
  BLOCKING_WAIT_REASONS,
  RESUMABLE_STATUSES,
  RUNNING_STATUSES,
  TERMINAL_STATUSES,
  WAITING_STATUSES,
  WORKFLOW_STATUSES,
  WORKFLOW_WAIT_REASONS
} from '@/composables/workflow/signalTypes';
import { deriveToolViewState } from '@/composables/workflow/useToolStateMapper';

/**
 * Task Ledger - 统一任务账本模型
 * 阶段9：建立 tool_call_id 统一视图模型，避免多轨状态冲突
 */
export const useWorkflowStore = defineStore('workflow', () => {
  const safeParseArguments = (raw) => {
    if (!raw) return {};
    if (typeof raw === 'string') {
      try {
        const parsed = JSON.parse(raw);
        return parsed && typeof parsed === 'object' ? parsed : {};
      } catch {
        return {};
      }
    }
    return typeof raw === 'object' ? raw : {};
  };

  const removeAssistantToolCallPlaceholder = (toolCallId) => {
    if (!toolCallId) return;

    messages.value = messages.value.flatMap((message) => {
      const toolCalls = message?.metadata?.tool_calls;
      if (message?.role !== 'assistant' || !Array.isArray(toolCalls) || toolCalls.length === 0) {
        return [message];
      }

      const remainingToolCalls = toolCalls.filter((call) => {
        const callId = call?.id || call?.tool_call_id;
        return callId !== toolCallId;
      });

      const hasText = Boolean((message.message || '').trim() || (message.reasoning || '').trim());
      if (remainingToolCalls.length === 0 && !hasText) {
        return [];
      }

      const nextMetadata = { ...(message.metadata || {}) };
      if (remainingToolCalls.length > 0) {
        nextMetadata.tool_calls = remainingToolCalls;
      } else {
        delete nextMetadata.tool_calls;
      }

      return [{
        ...message,
        metadata: nextMetadata
      }];
    });
  };

  // ==================== Core State ====================
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

  // ==================== Task Ledger State ====================
  /**
   * Task Ledger - 统一工具视图状态
   * 按 workflow ID 隔离存储，避免会话串台
   */
  const taskLedgerMap = ref(new Map()); // workflowId -> { tools: Map<toolCallId, ToolViewState>, lastUpdated }
  const taskLedgerEnabled = ref(true); // 功能开关，支持降级

  // ==================== Computed ====================
  const currentWorkflow = computed(() => {
    return workflows.value.find(w => w.id === currentWorkflowId.value);
  });

  const runningLikeStates = [...RUNNING_STATUSES];
  const waitingLikeStates = [...WAITING_STATUSES];
  const approvalWaitingStates = [...APPROVAL_WAITING_STATUSES];

  // ==================== Task Ledger Computed ====================

  /**
   * 当前会话的任务账本
   */
  const currentTaskLedger = computed(() => {
    const id = currentWorkflowId.value;
    if (!id) return null;

    if (!taskLedgerMap.value.has(id)) {
      taskLedgerMap.value.set(id, {
        tools: new Map(),
        lastUpdated: Date.now()
      });
    }
    return taskLedgerMap.value.get(id);
  });

  /**
   * 当前工具列表（按创建时间排序）
   */
  const toolList = computed(() => {
    if (!taskLedgerEnabled.value) return [];
    const ledger = currentTaskLedger.value;
    if (!ledger) return [];

    const tools = Array.from(ledger.tools.values());
    return tools.sort((a, b) => a.createdAt - b.createdAt);
  });

  /**
   * 按状态分组的工具
   */
  const toolsByStatus = computed(() => {
    const list = toolList.value;
    return {
      pending: list.filter(t => t.status === 'pending'),
      running: list.filter(t => t.status === 'approved_running'),
      completed: list.filter(t => t.status === 'final_success'),
      failed: list.filter(t => t.status === 'final_error' || t.status === 'rejected'),
      all: list
    };
  });

  /**
   * 任务账本进度统计
   */
  const progressStats = computed(() => {
    const total = toolList.value.length;
    const completed = toolsByStatus.value.completed.length;
    const failed = toolsByStatus.value.failed.length;
    const running = toolsByStatus.value.running.length;
    const pending = toolsByStatus.value.pending.length;
    const finished = completed + failed;
    const percent = total > 0 ? Math.round((finished / total) * 100) : 0;

    return { total, completed, failed, running, pending, finished, percent };
  });

  // ==================== Helper Functions ====================

  const normalizeExecutionContext = (ctx) => {
    if (!ctx || typeof ctx !== 'object') return null;
    return {
      ...ctx,
      waitReason: ctx.waitReason ?? ctx.wait_reason ?? null,
      currentContextTokens: ctx.currentContextTokens ?? ctx.current_context_tokens ?? null,
      pendingTools: ctx.pendingTools ?? ctx.pending_tools ?? [],
      waitingOnTaskId: ctx.waitingOnTaskId ?? ctx.waiting_on_task_id ?? null,
      childSessions: ctx.childSessions ?? ctx.child_sessions ?? []
    };
  };

  const getStructuredPendingApproval = (ctx) => {
    const normalized = normalizeExecutionContext(ctx);
    if (!normalized) return null;
    if (normalized.waitReason !== WORKFLOW_WAIT_REASONS.APPROVAL) return null;
    const pendingTool = normalized.pendingTools.find(tool => tool?.tool_call_id || tool?.toolCallId);
    if (!pendingTool) return null;
    return {
      toolCallId: pendingTool.toolCallId ?? pendingTool.tool_call_id ?? '',
      toolName: pendingTool.toolName ?? pendingTool.tool_name ?? '',
      details: pendingTool.details ?? '',
      displayType: pendingTool.displayType ?? pendingTool.display_type ?? ''
    };
  };

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
    const status = currentWorkflow.value?.status?.toLowerCase() || '';
    if (
      [
        WORKFLOW_STATUSES.COMPLETED,
        WORKFLOW_STATUSES.AWAITING_USER,
        WORKFLOW_STATUSES.AWAITING_APPROVAL,
        WORKFLOW_STATUSES.AWAITING_AUTO_APPROVAL
      ].includes(status)
    ) {
      return false;
    }
    if (BLOCKING_WAIT_REASONS.includes(waitReason.value)) {
      return false;
    }
    return RESUMABLE_STATUSES.includes(status);
  });

  const pendingApprovalMessage = computed(() => {
    return findLatestPendingApprovalMessage(messages.value);
  });

  const pendingApprovalRequest = computed(() => {
    const structured = getStructuredPendingApproval(currentWorkflow.value?.executionContext);
    if (structured) return structured;

    const legacy = pendingApprovalMessage.value;
    if (!legacy) return null;
    return {
      toolCallId: legacy?.metadata?.tool_call_id || '',
      toolName: legacy?.metadata?.tool_name || legacy?.metadata?.title || 'Tool Approval',
      details: legacy?.message || '',
      displayType: legacy?.metadata?.display_type || ''
    };
  });

  const canApprovePending = computed(() => {
    const status = currentWorkflow.value?.status?.toLowerCase() || '';
    const isApprovalWaiting =
      waitReason.value === WORKFLOW_WAIT_REASONS.APPROVAL || approvalWaitingStates.includes(status);
    if (!isApprovalWaiting) return false;
    return !!pendingApprovalRequest.value;
  });

  const canApprovePlan = computed(() => {
    const status = currentWorkflow.value?.status?.toLowerCase() || '';
    const isApprovalWaiting =
      waitReason.value === WORKFLOW_WAIT_REASONS.APPROVAL || approvalWaitingStates.includes(status);
    if (!isApprovalWaiting) return false;
    return !canApprovePending.value;
  });

  /**
   * Extract allowed shell commands from shellPolicy
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

  // ==================== Task Ledger Actions ====================

  /**
   * 从消息重建任务账本
   * 在切换会话或需要同步时调用
   */
  const rebuildTaskLedger = () => {
    if (!taskLedgerEnabled.value || !currentWorkflowId.value) return;

    try {
      const derived = deriveToolViewState(
        messages.value,
        {
          get: (id) => toolStreams.value.get(id)
        },
        currentWorkflowId.value
      );

      const ledger = currentTaskLedger.value;
      if (ledger) {
        ledger.tools = derived;
        ledger.lastUpdated = Date.now();
      }
    } catch (err) {
      console.error('[TaskLedger] Failed to rebuild:', err);
    }
  };

  /**
   * 更新或创建工具状态
   */
  const upsertToolViewState = (toolState) => {
    if (!taskLedgerEnabled.value || !currentWorkflowId.value) return null;

    const ledger = currentTaskLedger.value;
    if (!ledger) return null;

    const { toolCallId } = toolState;
    const existing = ledger.tools.get(toolCallId);
    const now = Date.now();

    // 状态优先级收敛
    const priority = {
      'final_error': 4,
      'final_success': 4,
      'rejected': 3,
      'approved_running': 2,
      'pending': 1
    };

    const newStatus = toolState.status || existing?.status || 'pending';
    const existingPriority = existing ? (priority[existing.status] || 0) : 0;
    const newPriority = priority[newStatus] || 0;

    // 如果现有状态优先级更高，保留现有状态
    if (existingPriority > newPriority && !toolState.force) {
      return existing;
    }

    const next = {
      toolCallId,
      toolName: toolState.toolName || existing?.toolName || 'unknown',
      status: newStatus,
      title: toolState.title || existing?.title || toolState.toolName || 'Tool',
      summary: toolState.summary ?? existing?.summary ?? 'Waiting...',
      arguments: toolState.arguments || existing?.arguments,
      result: toolState.result ?? existing?.result,
      errorType: toolState.errorType ?? existing?.errorType,
      approvalStatus: toolState.approvalStatus || existing?.approvalStatus || 'pending',
      createdAt: existing?.createdAt || now,
      updatedAt: now,
      workflowId: currentWorkflowId.value,
      streamOutput: toolState.streamOutput || existing?.streamOutput || [],
      isExpanded: toolState.isExpanded ?? existing?.isExpanded ?? false
    };

    ledger.tools.set(toolCallId, next);
    ledger.lastUpdated = now;

    return next;
  };

  const inferFinalExecutionStatus = (message, existingMeta = {}) => {
    const isError = message.isError || message.is_error || message.metadata?.is_error;
    const approvalStatus = message.metadata?.approval_status;

    if (approvalStatus === 'rejected') return 'rejected';
    if (isError) return 'failed';
    if (approvalStatus === 'pending') return 'pending_approval';

    // Incoming tool messages from backend are final observations.
    // The only non-final "approved" state is the optimistic frontend patch,
    // which does not go through addMessage.
    return 'completed';
  };

  /**
   * 标记工具为已批准
   */
  const markToolApprovedRunning = (toolId) => {
    if (!taskLedgerEnabled.value) {
      // 降级到旧逻辑
      patchToolMessage(toolId, (existing, meta) => ({
        ...existing,
        metadata: {
          ...meta,
          approval_status: 'approved',
          summary: meta.summary && meta.summary !== 'Awaiting approval'
            ? meta.summary
            : 'Executing...'
        }
      }));
      return;
    }

    const existing = currentTaskLedger.value?.tools.get(toolId);
    if (!existing) return;

    // 如果已经是终态，不再更新
    if (['final_success', 'final_error', 'rejected'].includes(existing.status)) {
      return;
    }

    upsertToolViewState({
      toolCallId: toolId,
      status: 'approved_running',
      approvalStatus: 'approved',
      summary: existing.summary === 'Awaiting approval' ? 'Executing...' : existing.summary
    });

    // 同时更新旧的消息元数据以保持兼容
    patchToolMessage(toolId, (existing, meta) => ({
      ...existing,
      message: '',
      metadata: {
        ...meta,
        approval_status: 'approved',
        execution_status: 'running',
        hide_approval_details: true,
        summary: meta.summary && meta.summary !== 'Awaiting approval'
          ? meta.summary
          : 'Executing...'
      }
    }));
  };

  /**
   * 追加工具流式输出
   */
  const appendToolStream = (toolId, line) => {
    // 更新传统流式存储
    if (!toolStreams.value.has(toolId)) {
      toolStreams.value.set(toolId, []);
    }
    const lines = toolStreams.value.get(toolId);
    lines.push(line);
    if (lines.length > 100) {
      lines.splice(0, lines.length - 100);
    }

    // 更新 Task Ledger
    if (taskLedgerEnabled.value) {
      const ledger = currentTaskLedger.value;
      const existing = ledger?.tools.get(toolId);
      if (existing) {
        const streamOutput = [...(existing.streamOutput || []), line];
        if (streamOutput.length > 100) {
          streamOutput.splice(0, streamOutput.length - 100);
        }

        const summaryLine = typeof line === 'string' ? line.trim() : '';
        upsertToolViewState({
          toolCallId: toolId,
          status: existing.status === 'pending' ? 'approved_running' : existing.status,
          approvalStatus: 'approved',
          streamOutput,
          summary: summaryLine.substring(0, 100) || existing.summary
        });
      }
    }

    // 同时更新旧的消息元数据
    const summaryLine = typeof line === 'string' ? line.trim() : '';
    if (summaryLine) {
      patchToolMessage(toolId, (existing, meta) => ({
        ...existing,
        message: '',
        metadata: {
          ...meta,
          approval_status: meta.approval_status === 'pending' ? 'approved' : meta.approval_status,
          execution_status: 'running',
          hide_approval_details: true,
          summary: summaryLine
        }
      }));
    }
  };

  /**
   * 清理指定 workflow 的 Task Ledger
   */
  const clearTaskLedger = (workflowId) => {
    taskLedgerMap.value.delete(workflowId);
  };

  /**
   * 切换 Task Ledger 功能开关
   */
  const setTaskLedgerEnabled = (enabled) => {
    taskLedgerEnabled.value = enabled;
    if (enabled && currentWorkflowId.value) {
      rebuildTaskLedger();
    }
  };

  // ==================== Other Actions ====================

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

  const setShellPolicy = (policy) => {
    shellPolicy.value = policy;
  };

  const removeShellPolicyItem = (pattern) => {
    const index = shellPolicy.value.findIndex(item => item.pattern === pattern);
    if (index > -1) {
      shellPolicy.value.splice(index, 1);
    }
  };

  const patchToolMessage = (toolId, patcher) => {
    if (!toolId) return;
    const index = messages.value.findIndex(
      (m) => m?.metadata?.tool_call_id === toolId
    );
    if (index === -1) return;

    const existing = messages.value[index];
    const existingMeta = existing.metadata || {};
    const next = patcher(existing, existingMeta);
    if (!next) return;
    messages.value[index] = next;
  };

  const clearToolStream = (toolId) => {
    toolStreams.value.delete(toolId);
  };

  const getToolStream = (toolId) => {
    return toolStreams.value.get(toolId) || [];
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
    throw err;
  };

  const _parseWorkflowData = (w) => {
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

    w.allowedPaths = w.agentConfig.allowedPaths || [];
    w.shellPolicy = w.agentConfig.shellPolicy || [];

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

    // 清理前一个会话的临时状态
    if (currentWorkflowId.value && currentWorkflowId.value !== workflowId) {
      // 保留 Task Ledger 数据以支持切换回退
    }

    currentWorkflowId.value = workflowId;
    messageQueue.value = [];
    error.value = null;

    try {
      const snapshot = await invokeWrapper('get_workflow_snapshot', { sessionId: workflowId });
      console.log('workflowStore: snapshot loaded', snapshot);

      _parseWorkflowData(snapshot.workflow);
      snapshot.workflow.executionContext = normalizeExecutionContext(snapshot.executionContext);

      setShellPolicy(snapshot.workflow.shellPolicy || []);

      const status = snapshot.workflow.status?.toLowerCase() || WORKFLOW_STATUSES.PENDING;
      hasLiveSession.value = snapshot.hasLiveSession || false;
      isRunning.value = RUNNING_STATUSES.includes(status) && hasLiveSession.value;

      waitReason.value = snapshot.workflow.waitReason || null;

      const parsedMessages = (snapshot.messages || []).map(m => {
        if (m.metadata && typeof m.metadata === 'string') {
          try {
            m.metadata = JSON.parse(m.metadata);
          } catch (e) {
            console.error('Failed to parse snapshot message metadata:', e);
          }
        }
        if (m.is_error !== undefined) {
          m.isError = m.is_error;
        }
        return m;
      });

      messages.value = parsedMessages;

      // 重建 Task Ledger
      if (taskLedgerEnabled.value) {
        rebuildTaskLedger();
      }

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

      try {
        const tools = await invokeWrapper('get_auto_approved_tools', { sessionId: workflowId });
        if (tools && Array.isArray(tools)) {
          autoApprovedTools.value = tools;
        } else {
          autoApprovedTools.value = [];
        }
      } catch (e) {
        console.log('Could not fetch auto-approved tools:', e);
        autoApprovedTools.value = [];
      }

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
      // 清理 Task Ledger
      clearTaskLedger(workflowId);
    }
  };

  const createWorkflow = async (userQuery, agentId, allowedPaths = []) => {
    error.value = null;
    try {
      const newWorkflowId = await invokeWrapper('create_workflow', {
        workflow: {
          userQuery,
          agentId,
          allowedPaths: allowedPaths
        }
      });

      await loadWorkflows();
      await selectWorkflow(newWorkflowId);

      return newWorkflowId;
    } catch (err) {
      await _handleError(err);
    }
  };

  const addMessageToQueue = (message) => {
    messageQueue.value.push({
      id: message.id || `local_queue_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`,
      content: message.content || '',
      status: message.status || 'queued',
      sent: !!message.sent,
      createdAt: message.createdAt || Date.now(),
    });
  };

  const markQueuedMessageSent = (id) => {
    if (!id) return;
    const index = messageQueue.value.findIndex((item) => item.id === id);
    if (index === -1) return;
    messageQueue.value[index] = {
      ...messageQueue.value[index],
      sent: true,
      status: 'queued',
    };
  };

  const removeQueuedMessage = (id) => {
    if (!id) return;
    messageQueue.value = messageQueue.value.filter((item) => item.id !== id);
  };

  const acknowledgeQueuedMessage = (id) => {
    if (!id) return;
    messageQueue.value = messageQueue.value.filter((item) => item.id !== id);
  };

  const shiftQueuedMessage = () => {
    if (!messageQueue.value.length) return;
    messageQueue.value.shift();
  };

  const setRunning = (running) => {
    isRunning.value = running;
  };

  const setHasLiveSession = (live) => {
    hasLiveSession.value = !!live;
    const status = currentWorkflow.value?.status?.toLowerCase() || '';
    isRunning.value = RUNNING_STATUSES.includes(status) && hasLiveSession.value;
  };

  const addMessage = (message) => {
    if (!message.metadata) {
      message.metadata = {};
    }

    if (message.role === 'tool' && message.metadata?.tool_call_id) {
      message.metadata = {
        ...message.metadata,
        execution_status: inferFinalExecutionStatus(message, message.metadata),
        hide_approval_details: message.metadata?.approval_status === 'approved'
      };

      if (message.metadata.tool_name === 'bash' || message.metadata.title?.toLowerCase?.().includes('bash')) {
        const isError = message.isError || message.is_error || message.metadata?.is_error;
        message.metadata.summary = isError ? 'Execution failed' : 'Execution completed';
      }
    }

    const incomingToolCallId = message.metadata?.tool_call_id;
    const incomingQueuedUserMessageId = message.metadata?.queued_user_message_id;
    const index = messages.value.findIndex((m) => {
      if (message.id && m.id === message.id) return true;
      if (incomingToolCallId && m.metadata?.tool_call_id === incomingToolCallId) return true;
      if (
        incomingQueuedUserMessageId &&
        m.metadata?.queued_user_message_id === incomingQueuedUserMessageId
      ) {
        return true;
      }
      return false;
    });

    if (index !== -1) {
      messages.value[index] = { ...messages.value[index], ...message };
    } else {
      messages.value.push(message);
    }

    if (incomingToolCallId && (message.role === 'tool' || (message.role === 'user' && message.stepType === 'observe'))) {
      removeAssistantToolCallPlaceholder(incomingToolCallId);
    }

    // 更新 Task Ledger
    if (taskLedgerEnabled.value && incomingToolCallId) {
      const status = message.metadata?.approval_status;
      const isError = message.isError || message.is_error || message.metadata?.is_error;

      if (message.role === 'tool') {
        // 工具执行结果 - 终态
        upsertToolViewState({
          toolCallId: incomingToolCallId,
          toolName: message.metadata?.tool_name || message.metadata?.tool_call?.name,
          status: isError ? 'final_error' : 'final_success',
          title: message.metadata?.title,
          summary: message.metadata?.summary || (isError ? 'Failed' : 'Completed'),
          result: message.message,
          errorType: isError ? 'execution_error' : undefined,
          approvalStatus: isError ? 'approved' : (status || 'approved')
        });
      } else if (message.role === 'assistant' && message.metadata?.tool_calls?.length) {
        // 新的工具调用 - pending 状态
        for (const call of message.metadata.tool_calls) {
          const toolCallId = call.id;
          if (!toolCallId) continue;

          const toolName = call.name || call.function?.name;
          const args = safeParseArguments(call.arguments || call.function?.arguments);

          upsertToolViewState({
            toolCallId,
            toolName,
            status: 'pending',
            title: message.metadata?.title || toolName,
            summary: 'Awaiting approval',
            arguments: args,
            approvalStatus: 'pending'
          });
        }
      }
    }

    if (message.role === 'user') {
      const queuedId = message.metadata?.queued_user_message_id;
      const queueStatus = message.metadata?.queue_status;
      if (queuedId && (queueStatus === 'queued' || queueStatus === 'applied')) {
        acknowledgeQueuedMessage(queuedId);
      } else if (queueStatus === 'queued' || queueStatus === 'applied') {
        shiftQueuedMessage();
      }
    }
  };

  const updateWorkflowStatus = async (workflowId, status, waitReasonValue = null) => {
    error.value = null;
    try {
      waitReason.value = waitReasonValue;

      const statusLower = status.toLowerCase();
      const localUpdateStates = [
        ...RUNNING_STATUSES,
        ...WAITING_STATUSES,
        ...TERMINAL_STATUSES
      ];

      if (localUpdateStates.includes(statusLower)) {
        const workflowIndex = workflows.value.findIndex(w => w.id === workflowId);
        if (workflowIndex !== -1) {
          workflows.value[workflowIndex].status = status;
          workflows.value[workflowIndex].waitReason = waitReasonValue;
        }

        isRunning.value = RUNNING_STATUSES.includes(statusLower) && hasLiveSession.value;
      } else {
        await invokeWrapper('update_workflow_status', { sessionId: workflowId, status });
      }
    } catch (err) {
      await _handleError(err);
    }
  };

  const setCurrentContextTokens = (workflowId, totalTokens) => {
    if (!workflowId) return;
    const workflowIndex = workflows.value.findIndex(w => w.id === workflowId);
    if (workflowIndex === -1) return;

    const workflow = workflows.value[workflowIndex];
    const executionContext = normalizeExecutionContext(workflow.executionContext) || {};
    workflows.value[workflowIndex].executionContext = {
      ...executionContext,
      currentContextTokens: typeof totalTokens === 'number' ? totalTokens : null
    };
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

      // 重建 Task Ledger
      if (taskLedgerEnabled.value) {
        rebuildTaskLedger();
      }
    } catch (err) {
      await _handleError(err);
    }
  };

  const clearCurrentWorkflow = () => {
    if (currentWorkflowId.value) {
      clearTaskLedger(currentWorkflowId.value);
    }
    currentWorkflowId.value = null;
    messages.value = [];
    todoList.value = [];
    isRunning.value = false;
    waitReason.value = null;
    hasLiveSession.value = false;
    autoApprovedTools.value = [];
    shellPolicy.value = [];
    messageQueue.value = [];
  };

  return {
    // State
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
    toolStreams,

    // Task Ledger State
    taskLedgerMap,
    taskLedgerEnabled,
    currentTaskLedger,
    toolList,
    toolsByStatus,
    progressStats,

    // Computed
    currentWorkflow,
    isActivelyRunning,
    isLiveWaiting,
    isOrphanWaiting,
    isWaiting,
    canStop,
    canContinue,
    pendingApprovalMessage,
    pendingApprovalRequest,
    canApprovePending,
    canApprovePlan,
    allowedShellCommands,

    // Actions
    setNotification,
    setAutoApprovedTools,
    removeAutoApprovedTool,
    setShellPolicy,
    removeShellPolicyItem,
    markToolApprovedRunning,
    appendToolStream,
    clearToolStream,
    getToolStream,
    setTodoList,

    // Task Ledger Actions
    rebuildTaskLedger,
    upsertToolViewState,
    clearTaskLedger,
    setTaskLedgerEnabled,

    // Core Actions
    loadWorkflows,
    selectWorkflow,
    createWorkflow,
    addMessage,
    addMessageToQueue,
    markQueuedMessageSent,
    removeQueuedMessage,
    acknowledgeQueuedMessage,
    shiftQueuedMessage,
    setRunning,
    setHasLiveSession,
    updateWorkflowStatus,
    setCurrentContextTokens,
    updateWorkflowAllowedPaths,
    updateWorkflowFinalAudit,
    loadMessages,
    clearCurrentWorkflow,
  };
});
