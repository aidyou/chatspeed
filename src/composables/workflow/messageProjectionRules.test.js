import assert from 'node:assert/strict'

import {
  buildWorkflowTaskGroups,
  collectSubAgentCompletions,
  dedupeQueuedUserMessageProjection,
  excludeLeadingManualClearContextMarkers,
  excludeLeadingWorkflowTaskBoundaryMessages,
  excludeManualClearContextMarkers,
  getStructuredWorkflowToolName,
  getWorkflowMessageWindowAnchorId,
  getWorkflowPersistedMessageId,
  getWorkflowToolGroupRenderOrders,
  hasVisibleWorkflowText,
  hasIncompleteWorkflowToolCallChain,
  hasOpenWorkflowTaskFrame,
  inferWorkflowToolExecutionStatus,
  isPendingApprovalEntryForTool,
  isWorkflowCompletionMessage,
  isWorkflowToolAwaitingExecution,
  isWorkflowToolRunningForDisplay,
  mergeManualClearContextMarkersIntoPreviousGroups,
  mergeWorkflowMessagePages,
  normalizeVisibleCompletionReport,
  projectWorkflowMessageList,
  reconcileWorkflowTaskWindowState,
  resolveAskUserResponse,
  resolveFinalReviewSubAgentCompletion,
  resolveWorkflowPhaseFromPlanningMode,
  selectVisibleWorkflowMessageWindow,
  selectVisibleWorkflowTaskGroups,
  shouldRenderSubAgentCard
} from './messageProjectionRules.js'

const interleavedToolGroupOrders = getWorkflowToolGroupRenderOrders(
  [{ groupOrder: 0 }, { groupOrder: 2 }],
  [{ groupOrder: 0.5 }, { groupOrder: 2.5 }]
)
assert.deepEqual(
  interleavedToolGroupOrders,
  {
    thoughtOrders: [0, 2],
    toolOrders: [1, 3]
  },
  'fractional source positions must become integer flex orders without losing thought/tool interleaving'
)
assert.ok(
  [...interleavedToolGroupOrders.thoughtOrders, ...interleavedToolGroupOrders.toolOrders].every(
    Number.isInteger
  ),
  'tool-group render orders must stay valid CSS flex-order integers'
)

assert.equal(
  hasIncompleteWorkflowToolCallChain([
    {
      id: 'assistant-declaration',
      role: 'assistant',
      metadata: { tool_calls: [{ id: 'tool-read', function: { name: 'read_file' } }] }
    },
    {
      id: 'tool-result',
      role: 'tool',
      metadata: { tool_call_id: 'tool-read', tool_name: 'read_file', execution_status: 'completed' }
    }
  ]),
  false,
  'a dedicated tool result with its assistant declaration is a complete display chain'
)
assert.equal(
  hasIncompleteWorkflowToolCallChain([
    {
      id: 'tool-result-without-declaration',
      role: 'tool',
      metadata: { tool_call_id: 'tool-missing', tool_name: 'read_file' }
    }
  ]),
  true,
  'a page beginning at a tool result must request its missing assistant declaration'
)
assert.equal(
  hasIncompleteWorkflowToolCallChain([
    {
      id: 'ask-user-answer-without-source',
      role: 'user',
      stepType: 'Observe',
      message: '<ask_user_response>[]</ask_user_response>',
      metadata: { ui_visibility: 'hide' }
    }
  ]),
  true,
  'a hidden ask_user answer must not be loaded without its source tool record'
)

const askUserResponsesByToolCallId = new Map([
  ['ask-user-current', '<ask_user_response>["canonical"]</ask_user_response>']
])
const legacyAskUserResponsesBySourceOrder = new Map([
  [7, '<ask_user_response>["legacy"]</ask_user_response>']
])
assert.equal(
  resolveAskUserResponse(
    {
      role: 'tool',
      sourceOrder: 7,
      metadata: { tool_name: 'ask_user', tool_call_id: 'ask-user-current' }
    },
    askUserResponsesByToolCallId,
    legacyAskUserResponsesBySourceOrder
  ),
  '<ask_user_response>["canonical"]</ask_user_response>',
  'canonical ask_user answers must associate by tool_call_id instead of transcript position'
)
assert.equal(
  resolveAskUserResponse(
    {
      role: 'tool',
      sourceOrder: 7,
      metadata: { tool_name: 'ask_user', tool_call_id: 'ask-user-unanswered' }
    },
    askUserResponsesByToolCallId,
    legacyAskUserResponsesBySourceOrder
  ),
  '',
  'a missing canonical answer must not fall back to a positionally adjacent legacy response'
)
assert.equal(
  resolveAskUserResponse(
    {
      role: 'tool',
      sourceOrder: 7,
      metadata: { tool_name: 'ask_user' }
    },
    askUserResponsesByToolCallId,
    legacyAskUserResponsesBySourceOrder
  ),
  '<ask_user_response>["legacy"]</ask_user_response>',
  'position matching remains a compatibility path only for historic rows without tool_call_id'
)

const projectMessageList = (messages, options = {}) =>
  projectWorkflowMessageList(messages, {
    removeSystemReminder: value => String(value || '').replace(/<SYSTEM_REMINDER>[\s\S]*?<\/SYSTEM_REMINDER>/gi, ''),
    translate: (key, values = {}) => `${key}:${values.count ?? ''}`,
    ...options
  })

assert.equal(
  hasVisibleWorkflowText('\u200B'),
  false,
  'zero-width formatting characters must not count as visible workflow text'
)
assert.equal(
  hasVisibleWorkflowText(' \t\n '),
  false,
  'whitespace-only workflow text must not count as visible content'
)
assert.equal(
  hasVisibleWorkflowText('  visible content  '),
  true,
  'visible workflow text may include surrounding whitespace'
)

const zeroWidthThoughtProjection = projectMessageList([
  {
    id: 'tool-before-zero-width-thought',
    role: 'tool',
    metadata: { tool_call_id: 'tool-before-zero-width-thought', tool_name: 'read_file' },
    toolDisplay: { summary: 'Read before' }
  },
  {
    id: 'zero-width-thought',
    role: 'assistant',
    stepType: 'Think',
    message: '\u200B',
    reasoning: ''
  },
  {
    id: 'tool-after-zero-width-thought',
    role: 'tool',
    metadata: { tool_call_id: 'tool-after-zero-width-thought', tool_name: 'read_file' },
    toolDisplay: { summary: 'Read after' }
  }
])
assert.equal(
  zeroWidthThoughtProjection.length,
  1,
  'an invisible Think message must not split adjacent tool activity'
)
assert.deepEqual(
  zeroWidthThoughtProjection[0].groupedTools.map(message => message.id),
  ['tool-before-zero-width-thought', 'tool-after-zero-width-thought'],
  'tool activity around an invisible Think message must remain in one group'
)

assert.deepEqual(
  projectMessageList({ messages: [] }),
  [],
  'a malformed message-list input must not prevent the workflow message list from rendering'
)

const projectedToolActivity = projectMessageList([
  { id: 'thought-1', role: 'assistant', reasoning: 'Inspect the current code', message: '' },
  {
    id: 'read-1',
    role: 'tool',
    groupOrder: 1,
    metadata: { tool_call_id: 'read-1', tool_name: 'read_file', execution_status: 'completed' },
    toolDisplay: { summary: 'Read src/App.vue' }
  },
  {
    id: 'search-1',
    role: 'tool',
    groupOrder: 2,
    metadata: { tool_call_id: 'search-1', tool_name: 'grep', execution_status: 'completed' },
    toolDisplay: { summary: 'Search render rules' }
  },
  { id: 'next-user', role: 'user', message: 'Continue' }
])
assert.equal(projectedToolActivity.length, 2)
assert.equal(projectedToolActivity[0].metadata.message_kind, 'tool_group')
assert.equal(projectedToolActivity[0].metadata.tool_group_is_ongoing, false)
assert.deepEqual(
  projectedToolActivity[0].groupedThoughts.map(message => message.id),
  ['thought-1'],
  'a thought followed by tool activity must project into that tool group'
)
assert.deepEqual(
  projectedToolActivity[0].groupedTools.map(message => message.id),
  ['read-1', 'search-1'],
  'contiguous read/search observations must retain their durable order inside one group'
)

const pendingApprovalProjection = projectMessageList(
  [
    {
      id: 'pending-write',
      role: 'tool',
      metadata: { tool_call_id: 'pending-write', tool_name: 'write_file', approval_status: 'pending' },
      toolDisplay: { summary: 'Write src/App.vue' }
    }
  ],
  { pendingApprovalIds: new Set(['pending-write']) }
)
assert.equal(
  pendingApprovalProjection[0].metadata.message_kind,
  undefined,
  'canonical pending approvals must remain standalone instead of joining a tool group'
)

const runningPendingProjection = projectMessageList([
  {
    id: 'assistant-with-pending-tools',
    role: 'assistant',
    message: '',
    reasoning: '',
    pendingToolCalls: [
      {
        id: 'pending-read',
        toolName: 'read_file',
        groupOrder: 0,
        summary: 'Read src/App.vue'
      }
    ]
  }
])
assert.equal(runningPendingProjection.length, 1)
assert.equal(runningPendingProjection[0].metadata.message_kind, 'tool_group')
assert.equal(runningPendingProjection[0].metadata.tool_group_is_ongoing, true)
assert.equal(
  runningPendingProjection[0].groupedTools[0].metadata.tool_call_id,
  'pending-read',
  'a pending auto-executed tool must use the same stable tool-call identity as its final observation'
)

const repeatedCompletionErrors = projectMessageList([
  {
    id: 'finish-error-1',
    role: 'tool',
    message: 'Completion rejected',
    metadata: { tool_name: 'complete_workflow' },
    toolDisplay: { isError: true, summary: 'Completion rejected' }
  },
  {
    id: 'finish-error-2',
    role: 'tool',
    message: 'Completion rejected',
    metadata: { tool_name: 'complete_workflow' },
    toolDisplay: { isError: true, summary: 'Completion rejected' }
  }
])
assert.equal(repeatedCompletionErrors.length, 1)
assert.equal(repeatedCompletionErrors[0].metadata.finish_task_error_count, 2)

const finalReviewPendingMessage = {
  metadata: {
    execution_status: 'waiting',
    review_display_state: 'final_review_pending',
    sub_agent_id: 'subagent_final_review_1'
  },
  subAgentCard: {
    status: 'running'
  }
}

assert.equal(
  inferWorkflowToolExecutionStatus(finalReviewPendingMessage, finalReviewPendingMessage.metadata),
  'waiting',
  'explicit backend waiting status must survive frontend projection'
)

assert.equal(
  shouldRenderSubAgentCard(finalReviewPendingMessage),
  true,
  'final review pending messages with a child-session id must render the delegated-task card'
)

assert.equal(
  shouldRenderSubAgentCard({
    metadata: {
      tool_name: 'complete_workflow'
    },
    subAgentCard: null
  }),
  false,
  'messages without an assembled sub-agent card must not render as delegated-task cards'
)

const finalReviewUsageSummary = {
  version: 1,
  terminal_status: 'completed',
  duration_ms: 120,
  self_usage: {
    input_tokens: 10,
    output_tokens: 5,
    cache_tokens: 0,
    total_tokens: 15,
    estimated_cost: 0.01,
    effective_cost_per_million: 666.67,
    unpriced_tokens: 0
  },
  with_sub_agents: {
    input_tokens: 10,
    output_tokens: 5,
    cache_tokens: 0,
    total_tokens: 15,
    estimated_cost: 0.01,
    effective_cost_per_million: 666.67,
    unpriced_tokens: 0
  },
  has_sub_agents: false,
  is_partial: false,
  model_breakdowns: []
}

const finalReviewCompletion = resolveFinalReviewSubAgentCompletion(
  {
    metadata: {
      tool_name: 'complete_workflow',
      sub_agent_id: 'subagent_final_review_1',
      review_display_state: 'final_review_completed',
      sub_agent_status: 'completed',
      review_result: {
        status: 'completed',
        result: '{"approved":true,"summary":"Ready"}',
        usage_summary: finalReviewUsageSummary
      }
    }
  },
  null
)
assert.equal(
  finalReviewCompletion.result.result,
  '{"approved":true,"summary":"Ready"}',
  'the completed final-review card must use its structured reviewer result'
)
assert.deepEqual(
  finalReviewCompletion.result.usage_summary,
  finalReviewUsageSummary,
  'the completed final-review card must retain the reviewer usage summary for COST'
)
assert.equal(
  resolveFinalReviewSubAgentCompletion(
    {
      metadata: {
        sub_agent_id: 'subagent_final_review_1',
        review_display_state: 'final_review_pending'
      }
    },
    null
  ),
  null,
  'a pending final review must not fabricate a terminal result or cost'
)

assert.equal(
  inferWorkflowToolExecutionStatus(
    {
      metadata: {
        approval_status: 'pending'
      }
    },
    {
      approval_status: 'pending'
    }
  ),
  'pending_approval',
  'pending approvals without an explicit execution status should still map to pending_approval'
)

assert.equal(
  isWorkflowToolAwaitingExecution(
    {
      metadata: {
        approval_status: 'approved',
        execution_status: 'approval_submitted'
      }
    },
    false
  ),
  true,
  'approval-submitted tools must render as awaiting execution before tool_started'
)

assert.equal(
  isWorkflowToolAwaitingExecution(
    {
      metadata: {
        approval_status: 'pending',
        execution_status: 'pending_approval'
      }
    },
    true
  ),
  true,
  'the local submission flag must cover the interval before approval metadata reconciliation'
)

assert.equal(
  isWorkflowToolAwaitingExecution(
    {
      metadata: {
        approval_status: 'approved',
        execution_status: 'running'
      }
    },
    true
  ),
  false,
  'the backend running state must take precedence over a stale local submission flag'
)

assert.equal(
  isWorkflowToolRunningForDisplay({
    metadata: { tool_name: 'ask_user', execution_status: 'waiting' }
  }),
  false,
  'ask_user waiting for a response must not render as a running tool'
)
assert.equal(
  isWorkflowToolRunningForDisplay({
    metadata: { tool_name: 'complete_workflow', execution_status: 'waiting' }
  }),
  true,
  'other workflow-owned waiting states must retain their active display state'
)
assert.equal(
  isWorkflowToolRunningForDisplay({
    metadata: { tool_name: 'ask_user', execution_status: 'running' }
  }),
  true,
  'an ask_user tool explicitly reported as running must still render as running'
)

assert.equal(
  isWorkflowToolAwaitingExecution(
    {
      metadata: {
        approval_status: 'rejected',
        execution_status: 'rejected'
      }
    },
    true
  ),
  false,
  'terminal backend states must take precedence over a stale local submission flag'
)

assert.equal(
  getStructuredWorkflowToolName({
    metadata: {
      title: 'Read write edit list bash grep glob web search Ask User FinishTask'
    }
  }),
  '',
  'display titles must never be interpreted as structured tool identity'
)

assert.equal(
  getStructuredWorkflowToolName({
    metadata: {
      tool_call: {
        function: {
          name: 'BASH'
        }
      },
      title: 'Submit Plan'
    }
  }),
  'bash',
  'structured tool identity must take precedence over unrelated display text'
)

assert.equal(
  isPendingApprovalEntryForTool(
    {
      id: 'tool_bash',
      sessionId: 'session-1',
      toolName: 'bash',
      action: 'Run a command containing submit plan'
    },
    'session-1',
    'submit_plan'
  ),
  false,
  'approval actions containing plan text must not be selected as submit_plan'
)

assert.equal(
  isPendingApprovalEntryForTool(
    {
      id: 'tool_plan',
      sessionId: 'session-1',
      toolName: 'submit_plan',
      action: 'Localized plan approval title'
    },
    'session-1',
    'submit_plan'
  ),
  true,
  'plan approval selection must use exact structured identity and session scope'
)

assert.equal(
  isWorkflowCompletionMessage(
    {
      metadata: {
        tool_name: 'bash',
        execution_status: 'pending_approval',
        approval_status: 'pending'
      },
      toolDisplay: {
        action:
          'Run sqlite3 chatspeed.db "SELECT InvalidFinishSummary, FinishTask FROM workflow_messages"'
      }
    }
  ),
  false,
  'bash commands containing Finish markers must keep their approval presentation'
)

assert.equal(
  isWorkflowCompletionMessage(
    {
      metadata: {
        tool_name: 'complete_workflow'
      },
      toolDisplay: {
        action: 'Finish task'
      }
    }
  ),
  true,
  'structured complete_workflow messages must use the completion presentation'
)

assert.equal(
  isWorkflowCompletionMessage(
    {
      metadata: {},
      toolDisplay: {
        action: 'Finish task'
      }
    }
  ),
  false,
  'messages without structured tool identity must never use completion presentation'
)

const queuedMessageProjection = dedupeQueuedUserMessageProjection([
  {
    id: null,
    role: 'user',
    message: 'queued input',
    metadata: {
      queued_user_message_id: 'queue-boundary-1',
      queue_status: 'queued'
    }
  },
  {
    id: 42,
    role: 'user',
    message: 'queued input',
    metadata: {
      queued_user_message_id: 'queue-boundary-1',
      queue_status: 'applied'
    }
  }
])
assert.equal(
  queuedMessageProjection.length,
  1,
  'one queued message must render once when completion-boundary reconciliation temporarily repeats it'
)
assert.equal(
  queuedMessageProjection[0].id,
  42,
  'the persisted applied record must replace its transient queued projection'
)

assert.equal(getWorkflowPersistedMessageId({ id: ' 42 ' }), '42')
assert.equal(getWorkflowPersistedMessageId({ id: 'temporary-message' }), null)

const mergedHistoryMessages = mergeWorkflowMessagePages(
  [
    { id: 40, message: 'older message' },
    { id: '41', message: 'stale live projection' }
  ],
  [
    { id: 41, message: 'current page projection' },
    { id: 42, message: 'current message' },
    { id: null, message: 'transient message' }
  ]
)
assert.deepEqual(
  mergedHistoryMessages.map(message => [message.id, message.message]),
  [
    [40, 'older message'],
    [41, 'current page projection'],
    [42, 'current message'],
    [null, 'transient message']
  ],
  'history page merging must be idempotent across numeric and string persisted IDs'
)

const unsafeIntegerCompletionId = Number('871982461364473856')
const unsafeIntegerMarkerId = Number('871982461364473857')
assert.equal(
  unsafeIntegerCompletionId,
  unsafeIntegerMarkerId,
  'the regression fixture must reproduce adjacent i64 ids colliding as JS numbers'
)
const collidingPersistedMessages = mergeWorkflowMessagePages(
  [
    {
      id: unsafeIntegerCompletionId,
      role: 'tool',
      message: 'Finished',
      metadata: { tool_name: 'complete_workflow', tool_call_id: 'tool_476bc7ec' }
    },
    {
      id: unsafeIntegerMarkerId,
      role: 'system',
      messageKind: 'summary',
      messageSubtype: 'manual_clear_context',
      message: ''
    }
  ],
  []
)
assert.deepEqual(
  collidingPersistedMessages.map(message => [message.role, message.metadata?.tool_name || message.messageSubtype]),
  [
    ['tool', 'complete_workflow'],
    ['system', 'manual_clear_context']
  ],
  'adjacent i64 id precision collisions must not let a clear-context marker replace complete_workflow'
)

const visibleCompletion = collectSubAgentCompletions(
  [
    {
      messages: [
        {
          metadata: {
            observation_type: 'sub_agent_completion',
            sub_agent_id: 'visible_background',
            execution_status: 'completed',
            result: { result: 'visible result' }
          }
        }
      ]
    }
  ],
  [
    {
      subAgentId: 'live_background',
      status: 'completed',
      result: { status: 'completed', result: 'live result' }
    }
  ]
)
assert.equal(visibleCompletion.get('visible_background').result.result, 'visible result')
assert.equal(visibleCompletion.get('live_background').result.result, 'live result')
assert.equal(
  visibleCompletion.has('hidden_history'),
  false,
  'completion projection must not scan messages outside visible task groups'
)

assert.equal(
  normalizeVisibleCompletionReport(
    '<THINK>Internal reasoning must not be rendered.</THINK>\nCompleted the requested change.\n<ThOuGhT>More internal reasoning.</ThOuGhT>\nVerified the targeted tests pass.'
  ),
  'Completed the requested change.\nVerified the targeted tests pass.',
  'completion report projection must remove mixed-case reasoning blocks before rendering'
)
assert.equal(
  normalizeVisibleCompletionReport('<thought>Reasoning only must not be rendered.</thought>'),
  '',
  'reasoning-only completion summaries must not render'
)

assert.equal(resolveWorkflowPhaseFromPlanningMode(true, 'implementation'), 'planning')
assert.equal(
  resolveWorkflowPhaseFromPlanningMode(false, 'implementation'),
  'implementation',
  'a programmatic planning toggle update must not downgrade active implementation to standard'
)
assert.equal(resolveWorkflowPhaseFromPlanningMode(false, 'planning'), 'standard')

const completedTaskGroup = {
  id: 'completed-task',
  isCompleted: true,
  messages: [{ id: 'completed-message', role: 'tool' }]
}
const clearContextMarker = {
  id: 'clear-context-marker',
  role: 'system',
  messageKind: 'summary',
  messageSubtype: 'manual_clear_context'
}
const activeTaskGroup = {
  id: 'active-task',
  isCompleted: false,
  messages: [clearContextMarker, { id: 'active-message', role: 'user' }]
}
const mergedTaskGroups = mergeManualClearContextMarkersIntoPreviousGroups(
  [completedTaskGroup, activeTaskGroup],
  messages => messages.map(message => message.id).join(':')
)

assert.deepEqual(
  mergedTaskGroups[0].messages.map(message => message.id),
  ['completed-message', 'clear-context-marker'],
  'the new-session marker must be merged into the preceding completed task group'
)
assert.deepEqual(
  mergedTaskGroups[1].messages.map(message => message.id),
  ['active-message'],
  'the active task group must not retain an orphan new-session marker'
)

const unifiedBoundaryGroups = buildWorkflowTaskGroups(
  [
    { id: 'task-a-content', role: 'user' },
    { id: 'task-a-completion', role: 'tool', metadata: { tool_name: 'complete_workflow' } },
    clearContextMarker,
    { id: 'task-b-content', role: 'assistant' },
    { ...clearContextMarker, id: 'task-b-clear-context' },
    { id: 'task-c-content', role: 'user' }
  ],
  { buildGroupId: messages => messages.map(message => message.id).join(':') }
)
assert.deepEqual(
  unifiedBoundaryGroups.map(group => group.messages.map(message => message.id)),
  [
    ['task-a-content', 'task-a-completion', 'clear-context-marker'],
    ['task-b-content', 'task-b-clear-context'],
    ['task-c-content']
  ],
  'completion and clear-context boundaries must close and remain attached to preceding content'
)
assert.deepEqual(
  buildWorkflowTaskGroups([
    { id: 'boundary-only-completion', role: 'tool', metadata: { tool_name: 'complete_workflow' } },
    clearContextMarker
  ]),
  [],
  'task boundary messages must never form a standalone task group'
)
assert.deepEqual(
  excludeManualClearContextMarkers([
    clearContextMarker,
    { id: 'active-message', role: 'user' }
  ]).map(message => message.id),
  ['active-message'],
  'the visible active projection must hide the marker even when its previous group is not loaded'
)
assert.deepEqual(
  excludeLeadingManualClearContextMarkers([
    clearContextMarker,
    { id: 'active-message', role: 'user' }
  ]).map(message => message.id),
  ['active-message'],
  'a new-session marker must not render between the history control and the first visible task'
)
assert.deepEqual(
  excludeLeadingManualClearContextMarkers([
    { id: 'completed-message', role: 'tool' },
    clearContextMarker,
    { id: 'active-message', role: 'user' }
  ]).map(message => message.id),
  ['completed-message', 'clear-context-marker', 'active-message'],
  'a new-session marker must remain visible between two visible tasks'
)
assert.deepEqual(
  excludeLeadingManualClearContextMarkers([
    { id: 'completed-message', role: 'tool' },
    clearContextMarker
  ]).map(message => message.id),
  ['completed-message', 'clear-context-marker'],
  'an expanded completed task must retain its trailing new-session marker'
)
assert.deepEqual(
  selectVisibleWorkflowTaskGroups([mergedTaskGroups[0]], mergedTaskGroups[1]),
  [mergedTaskGroups[1]],
  'the default one-task window must show only the active task'
)
assert.deepEqual(
  selectVisibleWorkflowTaskGroups([mergedTaskGroups[0]], mergedTaskGroups[1], 2),
  [mergedTaskGroups[0], mergedTaskGroups[1]],
  'an explicit history reveal must expand the window beyond its one-task default'
)
assert.deepEqual(
  selectVisibleWorkflowTaskGroups([completedTaskGroup], null),
  [completedTaskGroup],
  'the default one-task window must show the latest completed task when no task is active'
)
assert.deepEqual(
  selectVisibleWorkflowTaskGroups(
    [completedTaskGroup, { ...completedTaskGroup, id: 'newer-completed-task' }],
    null
  ).map(group => group.id),
  ['newer-completed-task'],
  'the default one-task window must show only the newest completed task'
)

const olderMessageGroup = {
  id: 'older-message-group',
  isCompleted: true,
  messages: [{ id: 'message-1' }, { id: 'message-2' }]
}
const currentMessageGroup = {
  id: 'current-message-group',
  isCompleted: false,
  messages: [{ id: 'message-3' }, { id: 'message-4' }, { id: 'message-5' }]
}
const threeMessageWindow = selectVisibleWorkflowMessageWindow(
  [olderMessageGroup, currentMessageGroup],
  3
)
assert.equal(threeMessageWindow.hiddenMessageCount, 2)
assert.deepEqual(threeMessageWindow.groups, [currentMessageGroup])
assert.equal(
  threeMessageWindow.groups[0],
  currentMessageGroup,
  'fully visible groups must retain their identity for projection cache reuse'
)

const fourMessageWindow = selectVisibleWorkflowMessageWindow(
  [olderMessageGroup, currentMessageGroup],
  4
)
assert.equal(fourMessageWindow.hiddenMessageCount, 1)
assert.deepEqual(
  fourMessageWindow.groups.flatMap(group => group.messages.map(message => message.id)),
  ['message-2', 'message-3', 'message-4', 'message-5'],
  'the message window must retain the newest messages across task boundaries'
)
assert.equal(
  fourMessageWindow.groups[1],
  currentMessageGroup,
  'groups after the sliced boundary must retain their identity'
)

const boundaryCompletion = {
  id: 'boundary-completion',
  role: 'tool',
  metadata: { tool_name: 'complete_workflow' }
}
const boundaryWindow = selectVisibleWorkflowMessageWindow(
  [
    {
      id: 'boundary-group',
      messages: [
        { id: 'hidden-message' },
        boundaryCompletion,
        clearContextMarker,
        ...Array.from({ length: 299 }, (_, index) => ({ id: `active-${index + 1}` }))
      ]
    }
  ],
  300
)
assert.equal(boundaryWindow.hiddenMessageCount, 3)
assert.deepEqual(
  boundaryWindow.groups[0].messages.slice(0, 2).map(message => message.id),
  ['active-1', 'active-2'],
  'task boundary messages must stay on the hidden side instead of forming a visible segment'
)

const longRunningTaskMessages = Array.from({ length: 5000 }, (_, index) => ({
  id: `long-task-message-${index + 1}`
}))
const longRunningTaskWindow = selectVisibleWorkflowMessageWindow(
  [{ id: 'long-running-task', isCompleted: false, messages: longRunningTaskMessages }],
  200
)
assert.equal(longRunningTaskWindow.hiddenMessageCount, 4800)
assert.equal(longRunningTaskWindow.groups[0].messages.length, 200)
assert.equal(
  longRunningTaskWindow.groups[0].messages[0].id,
  'long-task-message-4801',
  'long-running tasks must only project the bounded newest-message window'
)
assert.equal(
  longRunningTaskWindow.groups[0].messages[199].id,
  'long-task-message-5000'
)

const anchoredReadingMessages = Array.from({ length: 305 }, (_, index) => ({
  id: `anchored-message-${index + 1}`,
  role: 'assistant'
}))
const anchoredReadingWindow = selectVisibleWorkflowMessageWindow(
  [{ id: 'anchored-reading-task', isCompleted: false, messages: anchoredReadingMessages }],
  300,
  getWorkflowMessageWindowAnchorId(anchoredReadingMessages[1])
)
assert.equal(
  anchoredReadingWindow.hiddenMessageCount,
  1,
  'an off-bottom reading anchor must remain in the projected message window'
)
assert.deepEqual(
  anchoredReadingWindow.groups[0].messages.map(message => message.id).slice(0, 2),
  ['anchored-message-2', 'anchored-message-3'],
  'new messages may extend the reading window without removing the anchored message'
)
assert.equal(
  anchoredReadingWindow.groups[0].messages.length,
  304,
  'the anchored window must retain all messages from the reading anchor through the newest message'
)

const defaultAnchoredReadingWindow = selectVisibleWorkflowMessageWindow(
  [{ id: 'anchored-reading-task', isCompleted: false, messages: anchoredReadingMessages }],
  300,
  'missing-reading-anchor'
)
assert.equal(
  defaultAnchoredReadingWindow.hiddenMessageCount,
  5,
  'a stale anchor must fall back to the normal bounded latest-message window'
)
assert.equal(defaultAnchoredReadingWindow.groups[0].messages[0].id, 'anchored-message-6')

const createTaskWindowHarness = () => {
  const acceptedCompletionIds = new Set()
  let state
  const getToolCallId = message => message?.toolCallId || ''
  const getIdentity = (message, index) => String(message?.id || `message:${index}`)
  const buildGroupId = messages => messages.map(message => message.id).join(':')
  const isAcceptedCompletionMessage = message => message?.isAcceptedCompletion === true
  const buildTaskGroups = (messages, allowPersistedCompletionFallback = false) =>
    buildWorkflowTaskGroups(messages, {
      buildGroupId,
      preserveLeadingBoundaries: true,
      isCompletionBoundary: message =>
        acceptedCompletionIds.has(getToolCallId(message)) ||
        (allowPersistedCompletionFallback && isAcceptedCompletionMessage(message))
    })

  return {
    acceptedCompletionIds,
    reconcile(messages) {
      state = reconcileWorkflowTaskWindowState({
        messages,
        workflowId: 'workflow-1',
        state,
        acceptedCompletionIds,
        isAcceptedCompletionMessage,
        buildTaskGroups,
        buildGroupId,
        getMessageIdentity: getIdentity,
        getMessageToolCallId: getToolCallId
      })
      return state
    }
  }
}

const taskOneUser = { id: 'task-1-user', role: 'user' }
const taskOneCompletion = {
  id: 'task-1-completion',
  role: 'tool',
  toolCallId: 'completion-1',
  isAcceptedCompletion: true,
  metadata: { tool_name: 'complete_workflow' }
}
const taskTwoUser = { id: 'task-2-user', role: 'user' }

const completionMessageFirst = createTaskWindowHarness()
let incrementalState = completionMessageFirst.reconcile([taskOneUser])
incrementalState = completionMessageFirst.reconcile([taskOneUser, taskOneCompletion])
assert.equal(
  incrementalState.completedGroups.length,
  0,
  'a completion tool message must not rotate before the authoritative completion event arrives'
)
completionMessageFirst.acceptedCompletionIds.add('completion-1')
incrementalState = completionMessageFirst.reconcile([taskOneUser, taskOneCompletion])
assert.equal(incrementalState.completedGroups.length, 1)
assert.equal(incrementalState.activeMessages.length, 0)

const completionEventFirst = createTaskWindowHarness()
completionEventFirst.reconcile([taskOneUser])
completionEventFirst.acceptedCompletionIds.add('completion-1')
incrementalState = completionEventFirst.reconcile([taskOneUser])
assert.equal(incrementalState.completedGroups.length, 0)
incrementalState = completionEventFirst.reconcile([taskOneUser, taskOneCompletion])
assert.equal(
  incrementalState.completedGroups.length,
  1,
  'an earlier completion event must rotate once its durable tool message arrives'
)

incrementalState = completionEventFirst.reconcile([
  taskOneUser,
  taskOneCompletion,
  taskTwoUser
])
assert.deepEqual(
  selectVisibleWorkflowTaskGroups(
    incrementalState.completedGroups,
    {
      id: 'active-task',
      isCompleted: false,
      messages: incrementalState.activeMessages
    }
  )[0].messages,
  [taskTwoUser],
  'new active work must replace the completed task in the one-task window'
)

const markerAfterCompletion = { ...clearContextMarker }
incrementalState = completionEventFirst.reconcile([
  taskOneUser,
  taskOneCompletion,
  markerAfterCompletion
])
assert.equal(incrementalState.activeMessages.length, 0)
assert.equal(
  incrementalState.completedGroups[0].messages.filter(
    message => message.id === markerAfterCompletion.id
  ).length,
  1,
  'a locally inserted new-session marker must merge into the completed task exactly once'
)
assert.equal(
  hasOpenWorkflowTaskFrame(
    incrementalState.completedGroups,
    incrementalState.activeMessages
  ),
  true,
  'a trailing new-session marker must open an empty active task frame'
)
const clearedContextVisibleGroups = selectVisibleWorkflowTaskGroups(
  incrementalState.completedGroups,
  null,
  1,
  true
)
assert.deepEqual(
  clearedContextVisibleGroups,
  incrementalState.completedGroups,
  'an empty task frame must not consume the only visible task slot after clearing context'
)
assert.deepEqual(
  clearedContextVisibleGroups[0].messages.map(message => message.id),
  ['task-1-user', 'task-1-completion', 'clear-context-marker'],
  'the completed task call must remain immediately above its trailing new-session marker'
)

const refreshedMessages = [
  { ...taskOneUser },
  { ...taskOneCompletion },
  { ...markerAfterCompletion },
  { ...taskTwoUser }
]
incrementalState = completionEventFirst.reconcile(refreshedMessages)
assert.deepEqual(
  incrementalState.activeMessages.map(message => message.id),
  ['task-2-user'],
  'snapshot refresh must preserve the active task after the marker'
)
assert.equal(
  incrementalState.completedGroups[0].messages.filter(
    message => message.id === markerAfterCompletion.id
  ).length,
  1,
  'snapshot refresh must not duplicate the merged marker'
)
const activeBoundaryGroups = selectVisibleWorkflowTaskGroups(
  incrementalState.completedGroups,
  {
    id: 'active-task',
    isCompleted: false,
    messages: incrementalState.activeMessages
  },
  1,
  true
)
assert.deepEqual(
  activeBoundaryGroups.flatMap(group => group.messages.map(message => message.id)),
  ['task-2-user'],
  'adjacent complete_workflow and clear-context markers must stay in the previous task group'
)
const exactBoundaryWindow = selectVisibleWorkflowMessageWindow(
  [
    incrementalState.completedGroups[0],
    {
      ...activeBoundaryGroups[0],
      messages: [
        taskTwoUser,
        ...Array.from({ length: 298 }, (_, index) => ({ id: `task-2-message-${index + 2}` }))
      ]
    }
  ],
  300
)
assert.deepEqual(
  exactBoundaryWindow.groups
    .flatMap(group => group.messages)
    .slice(0, 3)
    .map(message => message.id),
  ['task-2-user', 'task-2-message-2', 'task-2-message-3'],
  'the 300-message limit must keep task boundaries attached to hidden earlier content'
)

const separatedClearContextBoundaryGroups = selectVisibleWorkflowTaskGroups(
  [
    {
      id: 'separated-clear-context-boundary',
      isCompleted: true,
      messages: [
        taskOneUser,
        taskOneCompletion,
        { id: 'message-before-clear-context', role: 'assistant' },
        markerAfterCompletion
      ]
    }
  ],
  {
    id: 'active-after-separated-boundary',
    isCompleted: false,
    messages: [taskTwoUser]
  },
  1,
  true
)
assert.deepEqual(
  separatedClearContextBoundaryGroups.flatMap(group =>
    group.messages.map(message => message.id)
  ),
  ['task-2-user'],
  'previous task boundaries must never be projected as a standalone group above active work'
)

const completedAfterClearContext = {
  id: 'task-2-completion',
  isCompleted: true,
  messages: [
    taskTwoUser,
    { id: 'task-2-tool', role: 'tool' },
    { id: 'task-2-completion-message', role: 'tool', metadata: { tool_name: 'complete_workflow' } }
  ]
}
const laterCompletedTask = {
  id: 'task-3-completed',
  isCompleted: true,
  messages: [{ id: 'task-3-user', role: 'user' }, { id: 'task-3-completion', role: 'tool', metadata: { tool_name: 'complete_workflow' } }]
}
const visibleUserQuestionGroups = selectVisibleWorkflowTaskGroups(
  [incrementalState.completedGroups[0], completedAfterClearContext, laterCompletedTask],
  null,
  2,
  false
)
assert.deepEqual(
  visibleUserQuestionGroups.flatMap(group => group.messages.map(message => message.id)).slice(0, 3),
  ['task-2-user', 'task-2-tool', 'task-2-completion-message'],
  'revealing later tasks must not extract an adjacent completion and clear marker as another group'
)

const earlierTaskUser = { id: 'task-0-user', role: 'user' }
const earlierTaskCompletion = {
  id: 'task-0-completion',
  role: 'tool',
  toolCallId: 'completion-0',
  isAcceptedCompletion: true
}
incrementalState = completionEventFirst.reconcile([
  earlierTaskUser,
  earlierTaskCompletion,
  ...refreshedMessages
])
assert.equal(incrementalState.completedGroups.length, 2)
assert.equal(incrementalState.lastCompletionIndex, 4)
assert.deepEqual(incrementalState.activeMessages.map(message => message.id), ['task-2-user'])
assert.equal(
  incrementalState.completedGroups
    .flatMap(group => group.messages)
    .filter(message => message.id === markerAfterCompletion.id).length,
  1,
  'prepending an earlier task must relocate the completion boundary without duplicating the marker'
)
assert.equal(
  incrementalState.completedGroups.length,
  2,
  'both loaded completed groups remain available for one-at-a-time history reveal'
)

const cancelledTaskHarness = createTaskWindowHarness()
const cancelledTaskState = cancelledTaskHarness.reconcile([
  taskOneUser,
  clearContextMarker
])
assert.equal(
  cancelledTaskState.completedGroups.length,
  1,
  'manual clear-context must close unfinished work as a completed display segment'
)
assert.deepEqual(
  cancelledTaskState.completedGroups[0].messages.map(message => message.id),
  ['task-1-user', 'clear-context-marker'],
  'manual clear-context must stay attached to the unfinished work before it'
)
assert.deepEqual(
  excludeLeadingWorkflowTaskBoundaryMessages(cancelledTaskState.activeMessages).map(
    message => message.id
  ),
  [],
  'manual clear-context must leave a new empty task frame after the closed segment'
)

const cancelledTaskAfterCompletedHistory = createTaskWindowHarness()
const cancelledTaskWithHistoryState = cancelledTaskAfterCompletedHistory.reconcile([
  earlierTaskUser,
  earlierTaskCompletion,
  taskOneUser,
  clearContextMarker,
  taskTwoUser
])
assert.equal(
  cancelledTaskWithHistoryState.completedGroups.length,
  2,
  'manual clear-context must preserve both earlier completed and manually closed task segments'
)
assert.deepEqual(
  cancelledTaskWithHistoryState.completedGroups[1].messages.map(message => message.id),
  ['task-1-user', 'clear-context-marker'],
  'the clear-context boundary must remain attached to the immediately preceding work'
)
assert.deepEqual(
  excludeLeadingWorkflowTaskBoundaryMessages(cancelledTaskWithHistoryState.activeMessages).map(
    message => message.id
  ),
  ['task-2-user'],
  'work after manual clear-context must start a separate active task segment'
)

const persistedFallbackHarness = createTaskWindowHarness()
const persistedCompletionWithoutToolCallId = {
  id: 'persisted-completion-without-tool-id',
  role: 'tool',
  isAcceptedCompletion: true
}
let persistedFallbackState = persistedFallbackHarness.reconcile([
  taskOneUser,
  persistedCompletionWithoutToolCallId
])
assert.equal(persistedFallbackState.completedGroups.length, 1)
assert.equal(persistedFallbackState.lastCompletionIndex, 1)
persistedFallbackState = persistedFallbackHarness.reconcile([
  taskOneUser,
  persistedCompletionWithoutToolCallId
])
assert.equal(
  persistedFallbackState.completedGroups.length,
  1,
  'a persisted completion without tool_call_id must remain completed after refresh'
)
persistedFallbackState = persistedFallbackHarness.reconcile([
  taskOneUser,
  persistedCompletionWithoutToolCallId,
  taskTwoUser
])
assert.deepEqual(
  persistedFallbackState.activeMessages.map(message => message.id),
  ['task-2-user'],
  'new work after a persisted completion without tool_call_id must be the only active task'
)

const reinitializedHarness = createTaskWindowHarness()
reinitializedHarness.reconcile([])
reinitializedHarness.acceptedCompletionIds.add('stale-completion')
const reinitializedState = reinitializedHarness.reconcile([
  { id: 'new-task-user', role: 'user' },
  { id: 'reused-tool-id', role: 'tool', toolCallId: 'stale-completion' }
])
assert.equal(
  reinitializedState.completedGroups.length,
  0,
  'reinitializing a message window must not reuse stale live completion events'
)

console.log('messageProjectionRules tests passed')
