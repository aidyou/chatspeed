import assert from 'node:assert/strict'

import {
  collectSubAgentCompletions,
  excludeLeadingManualClearContextMarkers,
  excludeManualClearContextMarkers,
  getStructuredWorkflowToolName,
  hasOpenWorkflowTaskFrame,
  inferWorkflowToolExecutionStatus,
  isPendingApprovalEntryForTool,
  isWorkflowCompletionMessage,
  isWorkflowToolAwaitingExecution,
  mergeManualClearContextMarkersIntoPreviousGroups,
  normalizeVisibleCompletionReport,
  reconcileWorkflowTaskWindowState,
  resolveWorkflowPhaseFromPlanningMode,
  selectVisibleWorkflowTaskGroups,
  shouldRenderSubAgentCard
} from './messageProjectionRules.js'

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

const createTaskWindowHarness = () => {
  const acceptedCompletionIds = new Set()
  let state
  const getToolCallId = message => message?.toolCallId || ''
  const getIdentity = (message, index) => String(message?.id || `message:${index}`)
  const buildGroupId = messages => messages.map(message => message.id).join(':')
  const isAcceptedCompletionMessage = message => message?.isAcceptedCompletion === true
  const buildTaskGroups = (messages, allowPersistedCompletionFallback = false) => {
    const groups = []
    let currentMessages = []

    const pushGroup = isCompleted => {
      if (!currentMessages.length) return
      groups.push({
        id: buildGroupId(currentMessages),
        isCompleted,
        messages: currentMessages
      })
      currentMessages = []
    }

    for (const message of messages) {
      currentMessages.push(message)
      const isBoundary =
        acceptedCompletionIds.has(getToolCallId(message)) ||
        (allowPersistedCompletionFallback && isAcceptedCompletionMessage(message))
      if (isBoundary) pushGroup(true)
    }
    pushGroup(false)
    return mergeManualClearContextMarkersIntoPreviousGroups(groups, buildGroupId)
  }

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
  isAcceptedCompletion: true
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
assert.deepEqual(
  selectVisibleWorkflowTaskGroups(
    incrementalState.completedGroups,
    null,
    1,
    true
  ),
  [],
  'clearing context must immediately hide the previous completed task and its marker'
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
assert.equal(incrementalState.lastCompletionIndex, 3)
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
  0,
  'a cancelled task must not be projected as completed without complete_workflow'
)
assert.deepEqual(
  excludeLeadingManualClearContextMarkers(cancelledTaskState.activeMessages).map(
    message => message.id
  ),
  ['task-1-user', 'clear-context-marker'],
  'a new-session marker after visible cancelled work must remain visible'
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
  1,
  'completed history must remain available to the earlier-task expansion control'
)
assert.deepEqual(
  excludeLeadingManualClearContextMarkers(cancelledTaskWithHistoryState.activeMessages).map(
    message => message.id
  ),
  ['task-1-user', 'clear-context-marker', 'task-2-user'],
  'cancelled work, its new-session marker, and following work must remain in the active task group'
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
