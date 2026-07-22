import { FrontendAppError, invokeWrapper } from '@/libs/tauri';
import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import { useSettingStore } from '@/stores/setting';
import {
  APPROVAL_WAITING_STATUSES,
  RESUMABLE_STATUSES,
  RUNNING_STATUSES,
  TERMINAL_STATUSES,
  WAITING_STATUSES,
  WORKFLOW_STATUSES,
  WORKFLOW_WAIT_REASONS
} from '@/composables/workflow/signalTypes';
import { deriveToolViewState } from '@/composables/workflow/useToolStateMapper';
import { isAutoExecuteWorkflowTool } from '@/composables/workflow/toolApproval';
import { getToolStatusSummary } from '@/composables/workflow/toolDisplay';
import { inferWorkflowToolExecutionStatus } from '@/composables/workflow/messageProjectionRules';

/**
 * Task Ledger - 统一任务账本模型
 * 阶段9：建立 tool_call_id 统一视图模型，避免多轨状态冲突
 */
export const useWorkflowStore = defineStore('workflow', () => {
  const settingStore = useSettingStore();

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

  const hasStreamingOutput = (toolId) => {
    if (!toolId) return false;
    return (toolStreams.value.get(toolId) || []).length > 0;
  };

  const normalizeWorkflowMessage = (message, fallbackSessionId = null) => {
    const normalized = {
      ...message,
      id: message?.id ?? message?.persistedMessageId ?? message?.message_id ?? null,
      sessionId: message?.sessionId || message?.session_id || fallbackSessionId || null,
      messageKind: message?.messageKind || message?.message_kind || 'message',
      messageSubtype: message?.messageSubtype || message?.message_subtype || null,
    };

    if (normalized.metadata && typeof normalized.metadata === 'string') {
      try {
        normalized.metadata = JSON.parse(normalized.metadata);
      } catch (e) {
        console.error('Failed to parse snapshot message metadata:', e);
      }
    }

    if (normalized.metadata && typeof normalized.metadata === 'object') {
      normalized.metadata = {
        ...normalized.metadata,
        ui_visibility:
          normalized.metadata.ui_visibility ?? normalized.metadata.uiVisibility,
        message_kind:
          normalized.metadata.message_kind ?? normalized.metadata.messageKind,
        error_type:
          normalized.metadata.error_type ?? normalized.metadata.errorType,
      };
    }

    if (normalized.messageKind === 'message' && normalized.metadata?.type === 'summary') {
      normalized.messageKind = 'summary';
    }
    if (!normalized.messageSubtype && typeof normalized.metadata?.subtype === 'string') {
      normalized.messageSubtype = normalized.metadata.subtype;
    }
    if (
      normalized.role === 'system' &&
      String(normalized.message || '').trim() === 'MANUAL_CLEAR_CONTEXT'
    ) {
      normalized.messageKind = 'summary';
      normalized.messageSubtype = 'manual_clear_context';
      normalized.message = '';
    }

    if (normalized.is_error !== undefined && normalized.isError === undefined) {
      normalized.isError = normalized.is_error;
    }

    return normalized;
  };

  const getPersistedMessageId = (message) => {
    const value = message?.id;
    if (value === null || value === undefined || value === '') return null;
    const normalized = String(value).trim();
    return /^\d+$/.test(normalized) ? normalized : null;
  };

  const comparePersistedMessageOrder = (left, right) => {
    const leftId = getPersistedMessageId(left);
    const rightId = getPersistedMessageId(right);
    if (!leftId && !rightId) return 0;
    if (!leftId) return 1;
    if (!rightId) return -1;

    try {
      const leftOrder = BigInt(leftId);
      const rightOrder = BigInt(rightId);
      return leftOrder < rightOrder ? -1 : leftOrder > rightOrder ? 1 : 0;
    } catch {
      return 0;
    }
  };

  // ==================== Core State ====================
  const workflows = ref([]);
  const currentWorkflowId = ref(null);
  const messages = ref([]);
  const messageWindowBeforeId = ref(null);
  const hiddenCompletedTaskCount = ref(0);
  const todoList = ref([]);
  const messageQueue = ref([]);
  const isRunning = ref(false);
  const waitReason = ref(null);
  const hasLiveSession = ref(false);
  const hasBlockingLiveSession = ref(false);
  const canRewindTail = ref(false);
  const error = ref(null);
  const notification = ref({
    message: '',
    category: 'info',
    timestamp: 0
  });
  const autoApprovedTools = ref([]);
  const shellPolicy = ref([]);
  const toolStreams = ref(new Map()); // tool_id -> string[] (max 100 lines)
  const subAgentProgress = ref(new Map()); // sub_agent_id -> lightweight parent UI projection
  const approvalSubmissions = ref(new Map()); // sessionId -> Set<toolCallId>
  const taskCompletionRevision = ref(0);
  let messageLoadRevision = 0;
  const lastTaskCompletion = ref(null);

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

  const displayQueueItems = computed(() => {
    const currentWorkflow = currentWorkflowId.value;
    const activeSubAgentQueue = Array.from(subAgentProgress.value.values())
      .filter((progress) => {
        const parentSessionId =
          progress.parentSessionId ?? progress.parent_session_id ?? currentWorkflow;
        if (currentWorkflow && parentSessionId && parentSessionId !== currentWorkflow) {
          return false;
        }

        const status = String(progress.status || '').toLowerCase();
        return !['completed', 'failed', 'cancelled'].includes(status);
      })
      .sort((left, right) => {
        const leftTime = Number(left.createdAtMs || left.updatedAtMs || left.updated_at_ms || 0);
        const rightTime = Number(right.createdAtMs || right.updatedAtMs || right.updated_at_ms || 0);
        return leftTime - rightTime;
      })
      .map((progress) => {
        const status = String(progress.status || '').toLowerCase();
        const workflowState = String(progress.workflowState || progress.workflow_state || '').toLowerCase();
        const waitReason = String(progress.waitReason || progress.wait_reason || '').toLowerCase();
        const agentName = progress.agentName || progress.agent_name || 'Sub-agent';
        const task = progress.task || '';
        const toolCallsCount = progress.toolCallsCount ?? progress.tool_calls_count ?? 0;

        let statusText = agentName;
        if (status === 'waiting') {
          statusText = waitReason ? `${agentName} · waiting (${waitReason})` : `${agentName} · waiting`;
        } else if (status === 'running') {
          statusText = toolCallsCount > 0 ? `${agentName} · running · ${toolCallsCount} tools` : `${agentName} · running`;
        } else if (status === 'pending') {
          statusText = `${agentName} · pending`;
        } else if (status === 'stopping') {
          statusText = `${agentName} · stopping`;
        } else if (workflowState) {
          statusText = `${agentName} · ${workflowState}`;
        }

        return {
          id: `sub_agent_progress_${progress.subAgentId || progress.sub_agent_id}`,
          content: task || agentName,
          status: 'sub_agent_progress',
          statusText,
          sent: true,
          acknowledged: true,
          attachedContext: null,
          metadata: {
            subAgentId: progress.subAgentId || progress.sub_agent_id || '',
            parentSessionId: progress.parentSessionId || progress.parent_session_id || '',
            queueKind: 'sub_agent_progress',
          },
          attachments: [],
          createdAt: progress.createdAtMs || progress.updatedAtMs || progress.updated_at_ms || Date.now(),
          removable: false,
          icon: 'task',
        };
      });

    return [...messageQueue.value, ...activeSubAgentQueue];
  });

  const persistLastSelectedWorkflowId = (workflowId) => {
    void settingStore.setSetting('workflowLastSelectedId', workflowId || '').catch((error) => {
      console.warn('[Workflow] Failed to persist last selected workflow id:', error);
    });
  };

  const runningLikeStates = [...RUNNING_STATUSES];
  const waitingLikeStates = [...WAITING_STATUSES];
  const approvalWaitingStates = [...APPROVAL_WAITING_STATUSES];

  const computeBlockingLiveSession = (status, live) => {
    if (!live) return false;
    const statusLower = String(status || '').toLowerCase();
    if (!statusLower) return false;
    if (statusLower === WORKFLOW_STATUSES.COMPLETED) return false;
    if (TERMINAL_STATUSES.includes(statusLower)) return false;
    if (statusLower === WORKFLOW_STATUSES.STOPPING) return true;
    if (runningLikeStates.includes(statusLower)) return true;
    if (waitingLikeStates.includes(statusLower)) return true;
    return false;
  };

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

  const cloneApprovalSubmissions = () => {
    const next = new Map();
    for (const [sessionId, toolIds] of approvalSubmissions.value.entries()) {
      next.set(sessionId, new Set(toolIds));
    }
    return next;
  };

  const markApprovalSubmitted = (sessionId, toolCallId) => {
    if (!sessionId || !toolCallId) return;
    const next = cloneApprovalSubmissions();
    const toolIds = next.get(sessionId) || new Set();
    toolIds.add(toolCallId);
    next.set(sessionId, toolIds);
    approvalSubmissions.value = next;
  };

  const clearApprovalSubmission = (sessionId, toolCallId) => {
    if (!sessionId || !toolCallId) return;
    const existing = approvalSubmissions.value.get(sessionId);
    if (!existing?.has(toolCallId)) return;

    const next = cloneApprovalSubmissions();
    const toolIds = next.get(sessionId);
    toolIds?.delete(toolCallId);
    if (!toolIds || toolIds.size === 0) {
      next.delete(sessionId);
    } else {
      next.set(sessionId, toolIds);
    }
    approvalSubmissions.value = next;
  };

  const clearApprovalSubmissionsForSession = (sessionId) => {
    if (!sessionId || !approvalSubmissions.value.has(sessionId)) return;
    const next = cloneApprovalSubmissions();
    next.delete(sessionId);
    approvalSubmissions.value = next;
  };

  const reconcilePendingApprovalSubmissions = (sessionId, pendingToolIds = []) => {
    if (!sessionId || pendingToolIds.length === 0) return;
    const existing = approvalSubmissions.value.get(sessionId);
    if (!existing || existing.size === 0) return;

    const next = cloneApprovalSubmissions();
    const toolIds = next.get(sessionId);
    let changed = false;

    for (const toolCallId of pendingToolIds) {
      if (toolIds?.delete(toolCallId)) {
        changed = true;
      }
    }

    if (!changed) return;

    if (!toolIds || toolIds.size === 0) {
      next.delete(sessionId);
    } else {
      next.set(sessionId, toolIds);
    }
    approvalSubmissions.value = next;
  };

  const isApprovalSubmitted = (sessionId, toolCallId) => {
    if (!sessionId || !toolCallId) return false;
    return approvalSubmissions.value.get(sessionId)?.has(toolCallId) || false;
  };

  const normalizeExecutionContext = (ctx) => {
    if (!ctx || typeof ctx !== 'object') return null;
    return {
      ...ctx,
      waitReason: ctx.waitReason ?? ctx.wait_reason ?? null,
      currentContextTokens: ctx.currentContextTokens ?? ctx.current_context_tokens ?? null,
      maxContextTokens: ctx.maxContextTokens ?? ctx.max_context_tokens ?? null,
      pendingTools: ctx.pendingTools ?? ctx.pending_tools ?? [],
      waitingOnSubAgentId: ctx.waitingOnSubAgentId ?? ctx.waiting_on_sub_agent_id ?? null,
      subAgentSessions: ctx.subAgentSessions ?? ctx.sub_agent_sessions ?? []
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
      arguments: pendingTool.arguments ?? null,
      details: pendingTool.details ?? null,
      displayType: pendingTool.displayType ?? pendingTool.display_type ?? ''
    };
  };

  const upsertSubAgentProgress = (progress = {}) => {
    const subAgentId = progress.subAgentId ?? progress.sub_agent_id;
    if (!subAgentId) return;

    const currentWorkflow = currentWorkflowId.value;
    const parentSessionId = progress.parentSessionId ?? progress.parent_session_id ?? currentWorkflow;
    if (currentWorkflow && parentSessionId && parentSessionId !== currentWorkflow) return;

    const nextProgress = new Map(subAgentProgress.value);
    const existing = nextProgress.get(subAgentId) || {};
    nextProgress.set(subAgentId, {
      ...existing,
      ...progress,
      subAgentId,
      parentSessionId,
      agentName: progress.agentName ?? progress.agent_name ?? existing.agentName ?? null,
      task: progress.task ?? existing.task ?? null,
      status: progress.status ?? existing.status ?? null,
      workflowState: progress.workflowState ?? progress.workflow_state ?? existing.workflowState ?? null,
      waitReason: progress.waitReason ?? progress.wait_reason ?? existing.waitReason ?? null,
      toolCallsCount: progress.toolCallsCount ?? progress.tool_calls_count ?? existing.toolCallsCount ?? 0,
      currentContextTokens: progress.currentContextTokens ?? progress.current_context_tokens ?? existing.currentContextTokens ?? null,
      maxContextTokens: progress.maxContextTokens ?? progress.max_context_tokens ?? existing.maxContextTokens ?? null,
      isError: progress.isError ?? progress.is_error ?? existing.isError ?? false,
      updatedAtMs: progress.updatedAtMs ?? progress.updated_at_ms ?? Date.now(),
      createdAtMs: existing.createdAtMs ?? progress.createdAtMs ?? progress.created_at_ms ?? progress.updatedAtMs ?? progress.updated_at_ms ?? Date.now()
    });
    subAgentProgress.value = nextProgress;
  };

  const clearSubAgentProgress = () => {
    subAgentProgress.value = new Map();
  };

  const findLatestPendingApprovalMessage = (list = []) => {
    const finalizedIds = new Set();
    for (let i = list.length - 1; i >= 0; i--) {
      const msg = list[i];
      if (msg?.role !== 'tool') continue;
      const meta = msg.metadata || {};
      const toolCallId = meta.tool_call_id;
      const approvalStatus = String(meta.approval_status || '').toLowerCase();
      const executionStatus = String(meta.execution_status || '').toLowerCase();
      if (!toolCallId) continue;
      if (
        approvalStatus === 'approved' ||
        approvalStatus === 'rejected' ||
        executionStatus === 'approval_submitted' ||
        executionStatus === 'running' ||
        executionStatus === 'rejected' ||
        executionStatus === 'completed' ||
        executionStatus === 'failed' ||
        executionStatus === 'interrupted' ||
        (approvalStatus && approvalStatus !== 'pending') ||
        (!approvalStatus && !executionStatus)
      ) {
        finalizedIds.add(toolCallId);
        continue;
      }
      if (
        (approvalStatus === 'pending' || executionStatus === 'pending_approval') &&
        !finalizedIds.has(toolCallId)
      ) {
        return msg;
      }
    }
    return null;
  };

  const getCurrentWorkflowWaitReason = () => {
    const executionContext = normalizeExecutionContext(currentWorkflow.value?.executionContext);
    return String(
      waitReason.value ??
        currentWorkflow.value?.waitReason ??
        currentWorkflow.value?.wait_reason ??
        executionContext?.waitReason ??
        executionContext?.wait_reason ??
        ''
    ).toLowerCase();
  };

  const isCurrentWorkflowApprovalWaiting = () => {
    const status = currentWorkflow.value?.status?.toLowerCase() || '';
    return (
      getCurrentWorkflowWaitReason() === WORKFLOW_WAIT_REASONS.APPROVAL ||
      approvalWaitingStates.includes(status)
    );
  };

  const getCurrentWorkflowPendingTools = () => {
    const executionContext = normalizeExecutionContext(currentWorkflow.value?.executionContext);
    const pendingTools = executionContext?.pendingTools || executionContext?.pending_tools || [];
    return Array.isArray(pendingTools) ? pendingTools : [];
  };

  const stringifyStructuredMessageContent = (value) => {
    if (typeof value === 'string') return value;
    if (value == null) return '';
    try {
      return JSON.stringify(value);
    } catch {
      return String(value);
    }
  };

  const buildStructuredPendingToolMetadata = (pendingTool = {}) => {
    const toolCallId = String(pendingTool?.toolCallId || pendingTool?.tool_call_id || '').trim();
    const toolName = String(pendingTool?.toolName || pendingTool?.tool_name || '').trim();
    const argumentsValue = pendingTool?.arguments ?? null;
    const details = pendingTool?.details ?? null;
    const displayType = pendingTool?.displayType || pendingTool?.display_type || '';

    return {
      tool_call_id: toolCallId,
      tool_name: toolName,
      tool_call: {
        id: toolCallId,
        function: {
          name: toolName,
          arguments: argumentsValue
        }
      },
      details,
      display_type: displayType,
      summary: getToolStatusSummary(toolName, 'pending', 'Awaiting approval'),
      approval_status: 'pending',
      execution_status: 'pending_approval'
    };
  };

  const buildInlineApprovalEntry = ({
    currentId,
    workflowTitle,
    toolCallId,
    structuredPending = null,
    meta = {}
  }) => {
    const toolName =
      structuredPending?.toolName ??
      structuredPending?.tool_name ??
      meta.tool_name ??
      meta.tool_call?.function?.name ??
      meta.tool_call?.name ??
      '';
    const argumentsValue =
      structuredPending?.arguments ??
      meta.tool_call?.function?.arguments ??
      meta.tool_call?.arguments ??
      null;
    const details = structuredPending?.details ?? meta.details ?? null;
    const displayType =
      structuredPending?.displayType ??
      structuredPending?.display_type ??
      meta.display_type ??
      '';

    return {
      key: `${currentId}:${toolCallId}`,
      id: toolCallId,
      sessionId: currentId,
      kind: 'approval',
      workflowTitle,
      action: toolName || meta.title || 'Tool Approval',
      toolCallId,
      toolName: toolName || meta.title || 'Tool Approval',
      arguments: argumentsValue,
      details,
      displayType,
      updatedAt: Date.now()
    };
  };

  const hasPendingToolObservationMessage = (list = [], toolCallId) => {
    const normalizedToolCallId = String(toolCallId || '').trim();
    if (!normalizedToolCallId) return false;

    return list.some((message) => {
      const meta = message?.metadata || {};
      if (String(meta.tool_call_id || '').trim() !== normalizedToolCallId) return false;
      return getToolApprovalState(message, meta) === 'pending';
    });
  };

  const getToolApprovalState = (message, meta = {}) => {
    const approvalStatus = String(meta.approval_status || '').toLowerCase();
    const executionStatus = String(meta.execution_status || '').toLowerCase();
    const isError = message?.isError || message?.is_error || meta.is_error;

    if (
      (approvalStatus === 'pending' || executionStatus === 'pending_approval') &&
      executionStatus !== 'approval_submitted' &&
      executionStatus !== 'running' &&
      executionStatus !== 'rejected' &&
      executionStatus !== 'completed' &&
      executionStatus !== 'failed' &&
      executionStatus !== 'interrupted'
    ) {
      return 'pending';
    }

    if (
      approvalStatus === 'approved' ||
      approvalStatus === 'rejected' ||
      executionStatus === 'approval_submitted' ||
      executionStatus === 'running' ||
      executionStatus === 'rejected' ||
      executionStatus === 'completed' ||
      executionStatus === 'failed' ||
      executionStatus === 'interrupted' ||
      isError
    ) {
      return 'resolved';
    }

    // A normal tool observation with the same tool_call_id is the final result
    // for old histories where approval metadata was not patched in place.
    if (message?.role === 'tool') {
      return 'resolved';
    }

    return null;
  };

  const deriveCurrentInlinePendingApprovals = () => {
    if (!currentWorkflowId.value || !isCurrentWorkflowApprovalWaiting()) {
      return [];
    }

    const currentId = currentWorkflowId.value;
    const submittedToolIds = approvalSubmissions.value.get(currentId) || new Set();
    const workflowTitle =
      currentWorkflow.value?.title || currentWorkflow.value?.userQuery || 'Untitled Workflow';
    const order = [];
    const latestById = new Map();
    const structuredPendingById = new Map();

    for (const pendingTool of getCurrentWorkflowPendingTools()) {
      const toolCallId = String(pendingTool?.toolCallId || pendingTool?.tool_call_id || '').trim();
      if (!toolCallId) continue;

      if (!structuredPendingById.has(toolCallId)) {
        order.push(toolCallId);
      }

      structuredPendingById.set(toolCallId, pendingTool);
    }

    for (const message of messages.value) {
      const messageSessionId = message?.sessionId || currentId;
      if (messageSessionId !== currentId) continue;

      const meta = message.metadata || {};
      const toolCallId = String(meta.tool_call_id || '').trim();
      if (!toolCallId) continue;

      const state = getToolApprovalState(message, meta);
      if (!state) continue;

      if (!latestById.has(toolCallId)) {
        order.push(toolCallId);
      }

      latestById.set(toolCallId, {
        state,
        meta
      });
    }

    return order
      .map((toolCallId) => {
        if (submittedToolIds.has(toolCallId)) return null;
        const structuredPending = structuredPendingById.get(toolCallId) || null;
        const latest = latestById.get(toolCallId);
        if (latest?.state === 'resolved') return null;
        if (!structuredPending && latest?.state !== 'pending') return null;
        const meta = latest.meta || {};
        return buildInlineApprovalEntry({
          currentId,
          workflowTitle,
          toolCallId,
          structuredPending,
          meta
        });
      })
      .filter(Boolean);
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

  const isStopping = computed(() => {
    const status = currentWorkflow.value?.status?.toLowerCase() || '';
    return status === WORKFLOW_STATUSES.STOPPING;
  });

  const canClearContext = computed(() => {
    if (!currentWorkflow.value?.id) return false;
    const executionContext = normalizeExecutionContext(currentWorkflow.value?.executionContext);
    const state = String(
      executionContext?.state || currentWorkflow.value?.status || WORKFLOW_STATUSES.PENDING
    ).toLowerCase();
    const waitReasonValue = String(
      executionContext?.waitReason ?? currentWorkflow.value?.waitReason ?? waitReason.value ?? ''
    ).toLowerCase();
    if (
      [
        WORKFLOW_WAIT_REASONS.APPROVAL,
        WORKFLOW_WAIT_REASONS.USER_INPUT,
        WORKFLOW_WAIT_REASONS.CONFIRMATION,
        WORKFLOW_WAIT_REASONS.SUB_AGENT
      ].includes(waitReasonValue)
    ) {
      return false;
    }
    return [
      WORKFLOW_STATUSES.PENDING,
      WORKFLOW_STATUSES.COMPLETED,
      WORKFLOW_STATUSES.ERROR,
      WORKFLOW_STATUSES.FAILED,
      WORKFLOW_STATUSES.CANCELLED
    ].includes(state);
  });

  const canStop = computed(() => {
    return hasLiveSession.value && !isStopping.value && (isActivelyRunning.value || isLiveWaiting.value);
  });

  const canContinue = computed(() => {
    if (!currentWorkflow.value?.id) return false;
    const status = currentWorkflow.value?.status?.toLowerCase() || '';
    const tailRewindKind = String(currentWorkflow.value?.tailRewindKind || '').trim();
    if (
      status === WORKFLOW_STATUSES.COMPLETED ||
      status === WORKFLOW_STATUSES.STOPPING
    ) {
      return false;
    }

    // Interactive waits should be resolved through their dedicated UI, not a generic resume button.
    if (
      [
        WORKFLOW_STATUSES.AWAITING_USER,
        WORKFLOW_STATUSES.AWAITING_APPROVAL,
        WORKFLOW_STATUSES.AWAITING_AUTO_APPROVAL
      ].includes(status)
    ) {
      return false;
    }

    if (['ask_user_wait', 'ask_user_answered'].includes(tailRewindKind)) {
      return false;
    }

    // While a live session is already waiting on the backend, resuming should go through
    // the dedicated signal path instead of spawning a new executor.
    if (hasLiveSession.value) {
      return false;
    }

    return [
      ...RUNNING_STATUSES,
      WORKFLOW_STATUSES.PAUSED,
      WORKFLOW_STATUSES.AWAITING_SUB_AGENT,
      ...RESUMABLE_STATUSES
    ].includes(status);
  });

  const pendingApprovalMessage = computed(() => {
    return findLatestPendingApprovalMessage(messages.value);
  });

  const pendingApprovalRequest = computed(() => {
    const activeInlineEntry = deriveCurrentInlinePendingApprovals().find(
      entry => String(entry?.toolName || '').toLowerCase() !== 'submit_plan'
    );
    if (activeInlineEntry) {
      return {
        toolCallId: activeInlineEntry.toolCallId || activeInlineEntry.id || '',
        toolName: activeInlineEntry.toolName || activeInlineEntry.action || 'Tool Approval',
        arguments: activeInlineEntry.arguments ?? null,
        details: activeInlineEntry.details ?? null,
        displayType: activeInlineEntry.displayType || ''
      };
    }

    const legacy = pendingApprovalMessage.value;
    if (!legacy) return null;
    const legacyDetails = legacy?.metadata?.details ?? null;
    const legacyArguments =
      legacy?.metadata?.tool_call?.function?.arguments ||
      legacy?.metadata?.tool_call?.arguments ||
      null;
    if (!legacy?.metadata?.tool_call_id || (!legacyDetails && !legacyArguments)) {
      return null;
    }
    return {
      toolCallId: legacy?.metadata?.tool_call_id || '',
      toolName: legacy?.metadata?.tool_name || legacy?.metadata?.title || 'Tool Approval',
      arguments: legacyArguments,
      details: legacyDetails,
      displayType: legacy?.metadata?.display_type || ''
    };
  });

  const currentInlinePendingApprovalIds = computed(() => {
    return deriveCurrentInlinePendingApprovals().map((item) => item.id);
  });

  const currentInlinePendingApprovals = computed(() => {
    return deriveCurrentInlinePendingApprovals();
  });

  const pendingPlanApprovalRequest = computed(() => {
    const activePlanEntry = deriveCurrentInlinePendingApprovals().find(
      entry => String(entry?.toolName || '').toLowerCase() === 'submit_plan'
    );
    if (activePlanEntry) {
      return {
        toolCallId: activePlanEntry.toolCallId || activePlanEntry.id || '',
        toolName: 'submit_plan',
        arguments: activePlanEntry.arguments ?? null,
        details: activePlanEntry.details ?? null,
        displayType: activePlanEntry.displayType || ''
      };
    }

    const legacy = pendingApprovalMessage.value;
    if (
      legacy?.metadata?.tool_call_id &&
      legacy?.metadata?.tool_name === 'submit_plan'
    ) {
      return {
        toolCallId: legacy.metadata.tool_call_id,
        toolName: 'submit_plan',
        arguments:
          legacy.metadata.tool_call?.function?.arguments ||
          legacy.metadata.tool_call?.arguments ||
          null,
        details: legacy.metadata.details ?? null,
        displayType: legacy.metadata.display_type || ''
      };
    }

    return null;
  });

  const canApprovePending = computed(() => {
    if (!isCurrentWorkflowApprovalWaiting()) return false;
    return !!pendingApprovalRequest.value;
  });

  const canApprovePlan = computed(() => {
    if (!isCurrentWorkflowApprovalWaiting()) return false;
    return !!pendingPlanApprovalRequest.value;
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
        const nextLedger = {
          ...ledger,
          tools: new Map(derived),
          lastUpdated: Date.now()
        };
        taskLedgerMap.value.set(currentWorkflowId.value, nextLedger);
        taskLedgerMap.value = new Map(taskLedgerMap.value);
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

    const nextTools = new Map(ledger.tools);
    nextTools.set(toolCallId, next);
    taskLedgerMap.value.set(currentWorkflowId.value, {
      ...ledger,
      tools: nextTools,
      lastUpdated: now
    });
    taskLedgerMap.value = new Map(taskLedgerMap.value);

    return next;
  };

  const inferLedgerToolStatus = (message) => {
    const meta = message.metadata || {};
    const executionStatus = meta.execution_status;
    const approvalStatus = meta.approval_status;
    const isError = message.isError || message.is_error || meta.is_error;

    if (executionStatus === 'pending_approval' || approvalStatus === 'pending') {
      return 'pending';
    }
    if (executionStatus === 'approval_submitted' || executionStatus === 'running') {
      return 'approved_running';
    }
    if (executionStatus === 'rejected' || approvalStatus === 'rejected') {
      return 'rejected';
    }
    if (executionStatus === 'failed' || executionStatus === 'interrupted' || isError) {
      return 'final_error';
    }
    return 'final_success';
  };

  const getLedgerStatusSummary = (toolName, status, fallbackSummary) => {
    if (fallbackSummary) return fallbackSummary;

    if (status === 'pending') {
      return getToolStatusSummary(toolName, 'pending', 'Awaiting approval');
    }
    if (status === 'approved_running') {
      return getToolStatusSummary(toolName, 'running', 'Executing...');
    }
    if (status === 'rejected') {
      return getToolStatusSummary(toolName, 'rejected', 'User rejected');
    }
    if (status === 'final_error') {
      return getToolStatusSummary(toolName, 'failed', 'Failed');
    }
    return getToolStatusSummary(toolName, 'success', 'Completed');
  };

  const markToolApprovalSubmitted = (toolId, toolName = 'unknown') => {
    if (!toolId) return;
    const existing = currentTaskLedger.value?.tools.get(toolId);

    if (taskLedgerEnabled.value) {
      upsertToolViewState({
        toolCallId: toolId,
        toolName: existing?.toolName || toolName,
        status: 'approved_running',
        approvalStatus: 'approved',
        summary: getToolStatusSummary(existing?.toolName || toolName, 'running', 'Executing...')
      });
    }

    patchToolMessage(toolId, (existingMessage, meta) => ({
      ...existingMessage,
      message: '',
      metadata: {
        ...meta,
        approval_status: 'approved',
        execution_status: 'approval_submitted',
        hide_approval_details: true,
        summary: getToolStatusSummary(
          meta.tool_name || meta.tool_call?.function?.name || meta.tool_call?.name,
          'running',
          'Executing...'
        )
      }
    }));
  };

  /**
   * 标记工具为已批准
   */
  const markToolApprovedRunning = (toolId, toolName = 'unknown') => {
    if (!taskLedgerEnabled.value) {
      // 降级到旧逻辑
      patchToolMessage(toolId, (existing, meta) => ({
        ...existing,
        metadata: {
          ...meta,
          approval_status: 'approved',
          execution_status: 'running',
          hide_approval_details: true,
          summary: getToolStatusSummary(
            meta.tool_name || meta.tool_call?.function?.name || meta.tool_call?.name,
            'running',
            'Executing...'
          )
        }
      }));
      return;
    }

    const existing = currentTaskLedger.value?.tools.get(toolId);
    if (existing && ['final_success', 'final_error', 'rejected'].includes(existing.status)) {
      return;
    }

    upsertToolViewState({
      toolCallId: toolId,
      toolName: existing?.toolName || toolName,
      status: 'approved_running',
      approvalStatus: 'approved',
      summary: getToolStatusSummary(
        existing?.toolName || toolName,
        'running',
        'Executing...'
      )
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
        summary: getToolStatusSummary(
          meta.tool_name || meta.tool_call?.function?.name || meta.tool_call?.name,
          'running',
          'Executing...'
        )
      }
    }));
  };

  const markToolRejected = (toolId, rejectionMessage = '') => {
    if (!toolId) return;
    const existing = currentTaskLedger.value?.tools.get(toolId);
    const trimmedRejectionMessage = String(rejectionMessage || '').trim();

    if (taskLedgerEnabled.value) {
      upsertToolViewState({
        toolCallId: toolId,
        status: 'rejected',
        approvalStatus: 'rejected',
        summary: getToolStatusSummary(
          existing?.toolName,
          'rejected',
          trimmedRejectionMessage || 'User rejected'
        )
      });
    }

    patchToolMessage(toolId, (existing, meta) => ({
      ...existing,
      message: trimmedRejectionMessage || existing.message,
      metadata: {
        ...meta,
        approval_status: 'rejected',
        execution_status: 'rejected',
        rejection_message: trimmedRejectionMessage || meta.rejection_message,
        summary: getToolStatusSummary(
          meta.tool_name || meta.tool_call?.function?.name || meta.tool_call?.name,
          'rejected',
          trimmedRejectionMessage || 'User rejected'
        )
      }
    }));
  };

  const finalizeToolExecution = (toolId, success, result, errorType) => {
    if (!toolId) return;
    const existing = currentTaskLedger.value?.tools.get(toolId);
    const statusSummary = success
      ? getToolStatusSummary(existing?.toolName, 'success', result || 'Completed')
      : getToolStatusSummary(existing?.toolName, 'failed', errorType || result || 'Failed');

    if (taskLedgerEnabled.value) {
      upsertToolViewState({
        toolCallId: toolId,
        status: success ? 'final_success' : 'final_error',
        approvalStatus: 'approved',
        result,
        errorType,
        summary: statusSummary
      });
    }

    patchToolMessage(toolId, (existingMessage, meta) => ({
      ...existingMessage,
      metadata: {
        ...meta,
        approval_status: 'approved',
        execution_status: success ? 'completed' : 'failed',
        error_type: success ? meta.error_type : (errorType || meta.error_type),
        summary: statusSummary
      }
    }));
  };

  const markToolPendingApproval = (toolId) => {
    if (!toolId) return;
    const existing = currentTaskLedger.value?.tools.get(toolId);

    if (taskLedgerEnabled.value) {
      upsertToolViewState({
        toolCallId: toolId,
        status: 'pending',
        approvalStatus: 'pending',
        summary: getToolStatusSummary(existing?.toolName, 'pending', 'Awaiting approval'),
        force: true
      });
    }

    patchToolMessage(toolId, (existing, meta) => ({
      ...existing,
      metadata: {
        ...meta,
        approval_status: 'pending',
        execution_status: 'pending_approval',
        hide_approval_details: false,
        summary: getToolStatusSummary(
          meta.tool_name || meta.tool_call?.function?.name || meta.tool_call?.name,
          'pending',
          'Awaiting approval'
        )
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
    taskLedgerMap.value = new Map(taskLedgerMap.value);
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
    let changed = false;
    messages.value = messages.value.map((message) => {
      if (message?.metadata?.tool_call_id !== toolId) {
        return message;
      }

      const existingMeta = message.metadata || {};
      const next = patcher(message, existingMeta);
      if (!next) return message;
      changed = true;
      return next;
    });

    if (!changed) return;
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
    const requestRevision = ++messageLoadRevision;

    // 清理前一个会话的临时状态
    if (currentWorkflowId.value && currentWorkflowId.value !== workflowId) {
      // 保留 Task Ledger 数据以支持切换回退
    }

    // Clear the previous projection before changing the session id. The workflow
    // message grouper uses synchronous watchers, so exposing old messages under a
    // new id can initialize the new session with the previous session's groups.
    messages.value = [];
    messageWindowBeforeId.value = null;
    hiddenCompletedTaskCount.value = 0;
    currentWorkflowId.value = workflowId;
    lastTaskCompletion.value = null;
    messageQueue.value = [];
    error.value = null;
    hasBlockingLiveSession.value = false;
    canRewindTail.value = false;
    clearSubAgentProgress();

    try {
      const snapshot = await invokeWrapper('get_workflow_snapshot', { sessionId: workflowId });
      console.debug('workflowStore: snapshot loaded', {
        workflowId,
        messageCount: snapshot?.messages?.length || 0,
        hasLiveSession: snapshot?.hasLiveSession === true
      });

      // A slower request for a previously selected workflow must not overwrite
      // the projection of the workflow that is currently active.
      if (currentWorkflowId.value !== workflowId || messageLoadRevision !== requestRevision) {
        return;
      }

      _parseWorkflowData(snapshot.workflow);
      snapshot.workflow.executionContext = normalizeExecutionContext(snapshot.executionContext);
      snapshot.workflow.canRewindTail = snapshot.canRewindTail === true;
      snapshot.workflow.tailRewindKind = snapshot.tailRewindKind || null;

      setShellPolicy(snapshot.workflow.shellPolicy || []);

      const status = snapshot.workflow.status?.toLowerCase() || WORKFLOW_STATUSES.PENDING;
      hasLiveSession.value = snapshot.hasLiveSession || false;
      hasBlockingLiveSession.value = snapshot.hasBlockingLiveSession === true;
      canRewindTail.value = snapshot.canRewindTail === true;
      isRunning.value = RUNNING_STATUSES.includes(status) && hasLiveSession.value;

      waitReason.value =
        snapshot.workflow.waitReason ||
        snapshot.workflow.wait_reason ||
        snapshot.workflow.executionContext?.waitReason ||
        snapshot.workflow.executionContext?.wait_reason ||
        null;

      const parsedMessages = (snapshot.messages || []).map(m =>
        normalizeWorkflowMessage(m, workflowId)
      );
      const structuredPendingTools =
        snapshot.workflow.executionContext?.pendingTools ||
        snapshot.workflow.executionContext?.pending_tools ||
        [];

      if (Array.isArray(structuredPendingTools) && structuredPendingTools.length > 0) {
        for (const pendingTool of structuredPendingTools) {
          const toolCallId = String(
            pendingTool?.toolCallId || pendingTool?.tool_call_id || ''
          ).trim();
          if (!toolCallId || hasPendingToolObservationMessage(parsedMessages, toolCallId)) {
            continue;
          }

          const metadata = buildStructuredPendingToolMetadata(pendingTool);
          parsedMessages.push({
            id: null,
            sessionId: workflowId,
            role: 'tool',
            message: stringifyStructuredMessageContent(metadata.details),
            reasoning: null,
            stepType: 'Observe',
            stepIndex: parsedMessages.length,
            isError: false,
            errorType: null,
            metadata
          });
        }
      }

      messages.value = parsedMessages;
      messageWindowBeforeId.value = snapshot.messageWindowBeforeId ?? null;
      hiddenCompletedTaskCount.value = Number(snapshot.hiddenCompletedTaskCount) || 0;
      clearApprovalSubmissionsForSession(workflowId);

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

      const selectedWorkflow = workflowIndex !== -1 ? workflows.value[workflowIndex] : snapshot.workflow;
      if (selectedWorkflow?.isAutomationRun !== true) {
        persistLastSelectedWorkflowId(workflowId);
      }
    } catch (err) {
      if (currentWorkflowId.value !== workflowId) {
        return;
      }
      await _handleError(err);
      messages.value = [];
      todoList.value = [];
      hasBlockingLiveSession.value = false;
      canRewindTail.value = false;
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
      statusText: typeof message.statusText === 'string' ? message.statusText : '',
      sent: !!message.sent,
      acknowledged: !!message.acknowledged,
      attachedContext: message.attachedContext || null,
      metadata: message.metadata || null,
      attachments: Array.isArray(message.attachments) ? message.attachments : [],
      createdAt: message.createdAt || Date.now(),
      removable: message.removable !== false,
    });
  };

  const markQueuedMessageSent = (id) => {
    if (!id) return;
    const index = messageQueue.value.findIndex((item) => item.id === id);
    if (index === -1) return;
    messageQueue.value[index] = {
      ...messageQueue.value[index],
      sent: true,
    };
  };

  const updateQueuedMessage = (id, updates = {}) => {
    if (!id) return;
    const index = messageQueue.value.findIndex((item) => item.id === id);
    if (index === -1) return;
    messageQueue.value[index] = {
      ...messageQueue.value[index],
      ...updates,
    };
  };

  const acknowledgeQueuedMessageSent = (id) => {
    if (!id) return;
    const index = messageQueue.value.findIndex((item) => item.id === id);
    if (index === -1) return;
    messageQueue.value[index] = {
      ...messageQueue.value[index],
      sent: true,
      acknowledged: true,
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
    hasBlockingLiveSession.value = computeBlockingLiveSession(status, hasLiveSession.value);
  };

  const addMessage = (message) => {
    if (!message.metadata) {
      message.metadata = {};
    }

    message = normalizeWorkflowMessage(message, currentWorkflowId.value);

    if (message.role === 'user') {
      const queuedId = message.metadata?.queued_user_message_id;
      const queueStatus = message.metadata?.queue_status;

      if (queueStatus === 'queued') {
        if (queuedId) {
          const existing = messageQueue.value.find((item) => item.id === queuedId);
          if (existing) {
            acknowledgeQueuedMessageSent(queuedId);
          } else {
            addMessageToQueue({
              id: queuedId,
              content: message.message || message.content || '',
              status: 'queued',
              sent: true,
              acknowledged: true,
            });
          }
        }
        return;
      }

      if (queueStatus === 'applied') {
        if (queuedId) {
          acknowledgeQueuedMessage(queuedId);
        } else {
          shiftQueuedMessage();
        }
      }
    }

    if (message.role === 'tool' && message.metadata?.tool_call_id) {
      const toolCallId = message.metadata.tool_call_id;
      const hasStream = hasStreamingOutput(toolCallId);
      const isError = message.isError || message.is_error || message.metadata?.is_error;
      const visibleContent = String(message.message || '').trim();
      message.metadata = {
        ...message.metadata,
        // Keep explicit backend non-terminal statuses such as final-review
        // waiting. Rewriting them to `completed` breaks completed-task window
        // rotation and hides the reviewer child-session lifecycle.
        execution_status: inferWorkflowToolExecutionStatus(message, message.metadata),
        hide_approval_details:
          message.metadata?.approval_status === 'approved' && !visibleContent
      };

      if (hasStream) {
        message.metadata.summary = isError
          ? 'workflow.executionFailed'
          : 'workflow.executionCompleted';
      }
    }

    const incomingToolCallId = message.metadata?.tool_call_id;
    const incomingQueuedUserMessageId = message.metadata?.queued_user_message_id;
    const index = messages.value.findIndex((m) => {
      if (message.id && m.id === message.id) return true;
      if (
        incomingToolCallId &&
        m.metadata?.tool_call_id === incomingToolCallId &&
        m.role === message.role
      ) {
        return true;
      }
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

    messages.value = [...messages.value].sort(comparePersistedMessageOrder);

    // 更新 Task Ledger
    if (taskLedgerEnabled.value) {
      const status = message.metadata?.approval_status;
      const isError = message.isError || message.is_error || message.metadata?.is_error;

      if (message.role === 'tool' && incomingToolCallId) {
        const toolStatus = inferLedgerToolStatus(message);
        const toolName = message.metadata?.tool_name || message.metadata?.tool_call?.name;

        // 工具执行结果 - 终态
        upsertToolViewState({
          toolCallId: incomingToolCallId,
          toolName,
          status: toolStatus,
          title: message.metadata?.title,
          summary: getLedgerStatusSummary(toolName, toolStatus, message.metadata?.summary),
          result: message.message,
          errorType: isError ? 'execution_error' : undefined,
          approvalStatus:
            status || (toolStatus === 'pending' ? 'pending' : toolStatus === 'rejected' ? 'rejected' : 'approved')
        });
      } else if (message.role === 'assistant' && message.metadata?.tool_calls?.length) {
        // 新的工具调用 - pending / auto-running 初始状态
        for (const call of message.metadata.tool_calls) {
          const toolCallId = call.id;
          if (!toolCallId) continue;

          const toolName = call.name || call.function?.name;
          const args = safeParseArguments(call.arguments || call.function?.arguments);
          const autoExecute = isAutoExecuteWorkflowTool(toolName);

          upsertToolViewState({
            toolCallId,
            toolName,
            status: autoExecute ? 'approved_running' : 'pending',
            title: message.metadata?.title || toolName,
            summary: getToolStatusSummary(
              toolName,
              autoExecute ? 'running' : 'pending',
              autoExecute ? 'Executing...' : 'Awaiting approval'
            ),
            arguments: args,
            approvalStatus: autoExecute ? 'approved' : 'pending'
          });
        }
      }
    }

  };

  const removeCurrentWorkflowMessages = (predicate) => {
    if (typeof predicate !== 'function') return;
    const sessionId = currentWorkflowId.value;
    if (!sessionId) return;

    messages.value = messages.value.filter((message) => {
      if (message?.sessionId && message.sessionId !== sessionId) {
        return true;
      }

      return !predicate(message);
    });
  };

  const updateWorkflowStatus = async (workflowId, status, waitReasonValue = null) => {
    error.value = null;
    try {
      const statusLower = status.toLowerCase();
      const localUpdateStates = [
        ...RUNNING_STATUSES,
        ...WAITING_STATUSES,
        WORKFLOW_STATUSES.STOPPING,
        ...TERMINAL_STATUSES
      ];

      if (localUpdateStates.includes(statusLower)) {
        const workflowIndex = workflows.value.findIndex(w => w.id === workflowId);
        if (workflowIndex !== -1) {
          workflows.value[workflowIndex].status = status;
          workflows.value[workflowIndex].waitReason = waitReasonValue;

          const executionContext =
            normalizeExecutionContext(workflows.value[workflowIndex].executionContext) || {};
          workflows.value[workflowIndex].executionContext = {
            ...executionContext,
            state: statusLower,
            waitReason: waitReasonValue,
            wait_reason: waitReasonValue
          };
        }

        if (workflowId === currentWorkflowId.value) {
          waitReason.value = waitReasonValue;
          isRunning.value = RUNNING_STATUSES.includes(statusLower) && hasLiveSession.value;
          hasBlockingLiveSession.value = computeBlockingLiveSession(
            statusLower,
            hasLiveSession.value
          );
        }
      } else {
        await invokeWrapper('update_workflow_status', { sessionId: workflowId, status });
      }
    } catch (err) {
      await _handleError(err);
    }
  };

  const setCurrentContextTokens = (workflowId, totalTokens, maxTokens = null) => {
    if (!workflowId) return;
    const workflowIndex = workflows.value.findIndex(w => w.id === workflowId);
    if (workflowIndex === -1) return;

    const workflow = workflows.value[workflowIndex];
    const executionContext = normalizeExecutionContext(workflow.executionContext) || {};
    workflows.value[workflowIndex].executionContext = {
      ...executionContext,
      currentContextTokens: typeof totalTokens === 'number' ? totalTokens : null,
      maxContextTokens: typeof maxTokens === 'number'
        ? maxTokens
        : executionContext.maxContextTokens ?? null
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

  const updateWorkflowTitleLocal = (workflowId, title) => {
    if (!workflowId || !title) return;
    const workflowIndex = workflows.value.findIndex(w => w.id === workflowId);
    if (workflowIndex !== -1) {
      workflows.value[workflowIndex] = {
        ...workflows.value[workflowIndex],
        title
      };
    }

    if (currentWorkflowId.value === workflowId && currentWorkflow.value) {
      currentWorkflow.value.title = title;
    }
  };

  const loadMessages = async (workflowId) => {
    console.log('workflowStore: loading messages for', workflowId);
    const requestRevision = ++messageLoadRevision;
    error.value = null;
    try {
      const snapshot = await invokeWrapper('get_workflow_snapshot', { sessionId: workflowId });
      if (currentWorkflowId.value !== workflowId || messageLoadRevision !== requestRevision) return;

      messages.value = (snapshot.messages || []).map(message =>
        normalizeWorkflowMessage(message, workflowId)
      );
      messageWindowBeforeId.value = snapshot.messageWindowBeforeId ?? null;
      hiddenCompletedTaskCount.value = Number(snapshot.hiddenCompletedTaskCount) || 0;

      // 重建 Task Ledger
      if (taskLedgerEnabled.value) {
        rebuildTaskLedger();
      }
    } catch (err) {
      await _handleError(err);
    }
  };

  const loadEarlierTaskGroup = async () => {
    const workflowId = currentWorkflowId.value;
    const beforeMessageId = messageWindowBeforeId.value;
    const requestRevision = messageLoadRevision;
    if (!workflowId || !beforeMessageId || hiddenCompletedTaskCount.value <= 0) return false;

    const window = await invokeWrapper('get_earlier_workflow_messages', {
      sessionId: workflowId,
      beforeMessageId
    });
    if (
      currentWorkflowId.value !== workflowId ||
      messageLoadRevision !== requestRevision ||
      messageWindowBeforeId.value !== beforeMessageId
    ) {
      return false;
    }

    const earlierMessages = (window.messages || []).map(message =>
      normalizeWorkflowMessage(message, workflowId)
    );
    messages.value = [...earlierMessages, ...messages.value];
    messageWindowBeforeId.value = window.beforeMessageId ?? beforeMessageId;
    hiddenCompletedTaskCount.value = Number(window.hiddenCompletedTaskCount) || 0;
    return earlierMessages.length > 0;
  };

  const clearCurrentWorkflow = () => {
    if (currentWorkflowId.value) {
      clearTaskLedger(currentWorkflowId.value);
      clearApprovalSubmissionsForSession(currentWorkflowId.value);
    }
    currentWorkflowId.value = null;
    lastTaskCompletion.value = null;
    persistLastSelectedWorkflowId('');
    messages.value = [];
    messageWindowBeforeId.value = null;
    hiddenCompletedTaskCount.value = 0;
    todoList.value = [];
    isRunning.value = false;
    waitReason.value = null;
    hasLiveSession.value = false;
    hasBlockingLiveSession.value = false;
    canRewindTail.value = false;
    autoApprovedTools.value = [];
    shellPolicy.value = [];
    messageQueue.value = [];
    clearSubAgentProgress();
  };

  const resetWorkflowUiProjection = (workflowId) => {
    if (!workflowId) return;
    clearTaskLedger(workflowId);
    clearApprovalSubmissionsForSession(workflowId);

    if (currentWorkflowId.value === workflowId) {
      clearSubAgentProgress();
    }
  };

  const recordTaskCompleted = (sessionId, toolCallId, segmentId) => {
    if (!sessionId || !toolCallId || currentWorkflowId.value !== sessionId) return;
    lastTaskCompletion.value = {
      sessionId,
      toolCallId,
      segmentId: Number(segmentId) || null
    };
    taskCompletionRevision.value += 1;
  };

  return {
    // State
    workflows,
    currentWorkflowId,
    messages,
    messageWindowBeforeId,
    hiddenCompletedTaskCount,
    todoList,
    messageQueue,
    isRunning,
    waitReason,
    hasLiveSession,
    hasBlockingLiveSession,
    canRewindTail,
    error,
    notification,
    autoApprovedTools,
    shellPolicy,
    toolStreams,
    subAgentProgress,
    displayQueueItems,
    approvalSubmissions,
    taskCompletionRevision,
    lastTaskCompletion,

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
    isStopping,
    canClearContext,
    canStop,
    canContinue,
    pendingApprovalMessage,
    pendingApprovalRequest,
    currentInlinePendingApprovalIds,
    currentInlinePendingApprovals,
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
    markToolApprovalSubmitted,
    markToolRejected,
    finalizeToolExecution,
    markToolPendingApproval,
    markApprovalSubmitted,
    clearApprovalSubmission,
    clearApprovalSubmissionsForSession,
    reconcilePendingApprovalSubmissions,
    isApprovalSubmitted,
    appendToolStream,
    clearToolStream,
    getToolStream,
    setTodoList,
    recordTaskCompleted,

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
    removeCurrentWorkflowMessages,
    addMessageToQueue,
    markQueuedMessageSent,
    updateQueuedMessage,
    acknowledgeQueuedMessageSent,
    removeQueuedMessage,
    acknowledgeQueuedMessage,
    shiftQueuedMessage,
    setRunning,
    setHasLiveSession,
    updateWorkflowStatus,
    setCurrentContextTokens,
    upsertSubAgentProgress,
    clearSubAgentProgress,
    updateWorkflowAllowedPaths,
    updateWorkflowFinalAudit,
    updateWorkflowTitleLocal,
    loadMessages,
    loadEarlierTaskGroup,
    clearCurrentWorkflow,
    resetWorkflowUiProjection,
  };
});
