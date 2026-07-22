const DEFAULT_APPROVAL_WAIT_REASON = 'approval';
const DEFAULT_PENDING_SUMMARY = 'Awaiting approval';

const normalizeObject = (value) => {
  return value && typeof value === 'object' ? value : {};
};

export const stringifyStructuredMessageContent = (value) => {
  if (typeof value === 'string') return value;
  if (value == null) return '';
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
};

export const normalizePendingTool = (pendingTool = {}) => {
  const normalized = normalizeObject(pendingTool);

  return {
    ...normalized,
    toolCallId: String(normalized.toolCallId ?? normalized.tool_call_id ?? '').trim(),
    toolName: String(normalized.toolName ?? normalized.tool_name ?? '').trim(),
    arguments: normalized.arguments ?? null,
    details: normalized.details ?? null,
    displayType: String(normalized.displayType ?? normalized.display_type ?? '').trim()
  };
};

export const normalizeExecutionContextForApproval = (ctx) => {
  const normalized = normalizeObject(ctx);
  if (!Object.keys(normalized).length) return null;

  const rawPendingTools = Array.isArray(normalized.pendingTools)
    ? normalized.pendingTools
    : Array.isArray(normalized.pending_tools)
      ? normalized.pending_tools
      : [];

  return {
    ...normalized,
    waitReason: String(normalized.waitReason ?? normalized.wait_reason ?? '').trim() || null,
    pendingTools: rawPendingTools.map(normalizePendingTool)
  };
};

export const buildStructuredPendingToolMetadata = (
  pendingTool = {},
  { summary = DEFAULT_PENDING_SUMMARY } = {}
) => {
  const normalized = normalizePendingTool(pendingTool);

  return {
    tool_call_id: normalized.toolCallId,
    tool_name: normalized.toolName,
    tool_call: {
      id: normalized.toolCallId,
      function: {
        name: normalized.toolName,
        arguments: normalized.arguments
      }
    },
    details: normalized.details,
    display_type: normalized.displayType,
    summary,
    approval_status: 'pending',
    execution_status: 'pending_approval'
  };
};

export const getToolApprovalState = (message, meta = {}) => {
  const normalizedMeta = normalizeObject(meta);
  const approvalStatus = String(normalizedMeta.approval_status || '').toLowerCase();
  const executionStatus = String(normalizedMeta.execution_status || '').toLowerCase();
  const isError = message?.isError || message?.is_error || normalizedMeta.is_error;

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

  if (message?.role === 'tool') {
    return 'resolved';
  }

  return null;
};

export const hasPendingToolObservationMessage = (messages = [], toolCallId) => {
  const normalizedToolCallId = String(toolCallId || '').trim();
  if (!normalizedToolCallId) return false;

  return messages.some((message) => {
    const meta = normalizeObject(message?.metadata);
    if (String(meta.tool_call_id || '').trim() !== normalizedToolCallId) return false;
    return getToolApprovalState(message, meta) === 'pending';
  });
};

const buildInlineApprovalEntry = ({
  currentWorkflowId,
  workflowTitle,
  toolCallId,
  structuredPending = null,
  meta = {}
}) => {
  const normalizedMeta = normalizeObject(meta);
  const pending = structuredPending ? normalizePendingTool(structuredPending) : null;
  const toolName =
    pending?.toolName ||
    String(
      normalizedMeta.tool_name ??
      normalizedMeta.tool_call?.function?.name ??
      normalizedMeta.tool_call?.name ??
      ''
    ).trim();
  const argumentsValue =
    pending?.arguments ??
    normalizedMeta.tool_call?.function?.arguments ??
    normalizedMeta.tool_call?.arguments ??
    null;
  const details = pending?.details ?? normalizedMeta.details ?? null;
  const displayType = pending?.displayType || String(normalizedMeta.display_type || '').trim();

  return {
    key: `${currentWorkflowId}:${toolCallId}`,
    id: toolCallId,
    sessionId: currentWorkflowId,
    kind: 'approval',
    workflowTitle,
    action: toolName || normalizedMeta.title || 'Tool Approval',
    toolCallId,
    toolName: toolName || normalizedMeta.title || 'Tool Approval',
    arguments: argumentsValue,
    details,
    displayType,
    updatedAt: Date.now()
  };
};

export const deriveInlinePendingApprovals = ({
  currentWorkflowId,
  workflowTitle = 'Untitled Workflow',
  status = '',
  waitReason = null,
  executionContext = null,
  messages = [],
  submittedToolIds = new Set(),
  approvalWaitingStatuses = [],
  approvalWaitReason = DEFAULT_APPROVAL_WAIT_REASON
}) => {
  if (!currentWorkflowId) return [];

  const normalizedStatus = String(status || '').toLowerCase();
  const normalizedWaitReason = String(waitReason || '').toLowerCase();
  const isApprovalWaiting =
    normalizedWaitReason === approvalWaitReason ||
    approvalWaitingStatuses.includes(normalizedStatus);

  if (!isApprovalWaiting) {
    return [];
  }

  const normalizedContext = normalizeExecutionContextForApproval(executionContext);
  const pendingTools = normalizedContext?.pendingTools || [];
  const order = [];
  const latestById = new Map();
  const structuredPendingById = new Map();

  for (const pendingTool of pendingTools) {
    if (!pendingTool.toolCallId) continue;
    if (!structuredPendingById.has(pendingTool.toolCallId)) {
      order.push(pendingTool.toolCallId);
    }
    structuredPendingById.set(pendingTool.toolCallId, pendingTool);
  }

  for (const message of messages) {
    const messageSessionId = message?.sessionId || currentWorkflowId;
    if (messageSessionId !== currentWorkflowId) continue;

    const meta = normalizeObject(message?.metadata);
    const toolCallId = String(meta.tool_call_id || '').trim();
    if (!toolCallId) continue;

    const state = getToolApprovalState(message, meta);
    if (!state) continue;

    if (!latestById.has(toolCallId)) {
      order.push(toolCallId);
    }

    latestById.set(toolCallId, { state, meta });
  }

  return order
    .map((toolCallId) => {
      if (submittedToolIds.has(toolCallId)) return null;

      const structuredPending = structuredPendingById.get(toolCallId) || null;
      const latest = latestById.get(toolCallId);

      if (latest?.state === 'resolved') return null;
      if (!structuredPending && latest?.state !== 'pending') return null;

      return buildInlineApprovalEntry({
        currentWorkflowId,
        workflowTitle,
        toolCallId,
        structuredPending,
        meta: latest?.meta || {}
      });
    })
    .filter(Boolean);
};

export const appendMissingPendingToolMessages = ({
  messages = [],
  sessionId,
  executionContext = null,
  getPendingSummary
}) => {
  const nextMessages = Array.isArray(messages) ? [...messages] : [];
  const normalizedContext = normalizeExecutionContextForApproval(executionContext);
  const pendingTools = normalizedContext?.pendingTools || [];

  for (const pendingTool of pendingTools) {
    if (!pendingTool.toolCallId) continue;
    if (hasPendingToolObservationMessage(nextMessages, pendingTool.toolCallId)) continue;

    const summary =
      typeof getPendingSummary === 'function'
        ? getPendingSummary(pendingTool.toolName)
        : DEFAULT_PENDING_SUMMARY;

    const metadata = buildStructuredPendingToolMetadata(pendingTool, { summary });
    nextMessages.push({
      id: null,
      sessionId,
      role: 'tool',
      message: stringifyStructuredMessageContent(metadata.details),
      reasoning: null,
      stepType: 'Observe',
      stepIndex: nextMessages.length,
      isError: false,
      errorType: null,
      metadata
    });
  }

  return nextMessages;
};

export const detectApprovalRecoveryDrift = ({
  status = '',
  waitReason = null,
  executionContext = null,
  inlinePendingApprovals = [],
  approvalWaitingStatuses = [],
  approvalWaitReason = DEFAULT_APPROVAL_WAIT_REASON
}) => {
  const normalizedStatus = String(status || '').toLowerCase();
  const normalizedContext = normalizeExecutionContextForApproval(executionContext);
  const normalizedWaitReason = String(
    waitReason ?? normalizedContext?.waitReason ?? ''
  ).toLowerCase();
  const isApprovalWaiting =
    normalizedWaitReason === approvalWaitReason ||
    approvalWaitingStatuses.includes(normalizedStatus);

  if (!isApprovalWaiting) return null;

  const pendingTools = normalizedContext?.pendingTools || [];
  if (pendingTools.length === 0) return null;
  if (Array.isArray(inlinePendingApprovals) && inlinePendingApprovals.length > 0) return null;

  return {
    status: normalizedStatus,
    waitReason: normalizedWaitReason,
    pendingToolIds: pendingTools.map(tool => tool.toolCallId).filter(Boolean)
  };
};
